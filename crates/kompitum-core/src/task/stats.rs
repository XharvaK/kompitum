//! `kom task stats`: where a run's tokens went, step by step.

use crate::api::{TaskStatsParams, TaskStatsResult};
use crate::config::Config;
use crate::rpc::RpcError;
use crate::runtime::Core;
use crate::task::types::{Usage, cut};
use crate::task::worker::{self, Cli};
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// What a CLI without per-step numbers says instead.
const NO_STEPS: &str = "per-step numbers: not in this CLI's stream";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Step {
    pub context: u64,
    pub input: u64,
    pub output: u64,
    pub reasoning: u64,
    pub cache_read: u64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Call {
    pub name: String,
    pub target: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AgentCall {
    pub id: String,
    /// Transcript paths or session ids found in the result that answers this call.
    pub candidates: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Parsed {
    pub steps: Vec<Step>,
    pub calls: Vec<Vec<Call>>,
    /// OpenCode subagent session ids.
    pub children: Vec<String>,
    /// Command Code `agent` calls.
    pub agents: Vec<AgentCall>,
    /// Steps a Command Code compaction line marks, counted from zero.
    pub marked: Vec<usize>,
    pub note: Option<String>,
    pub parse_errors: u64,
}

#[derive(Serialize, JsonSchema, Clone, Debug, Default, PartialEq)]
pub struct KindStats {
    /// The tool's name, or start, compaction or reply.
    pub kind: String,
    /// Tokens this kind added; a small drop is negative.
    pub growth: i64,
    /// Growth times the steps it stayed in the context.
    pub cost: i64,
}

/// One of the steps that added most to the context.
#[derive(Serialize, JsonSchema, Clone, Debug, Default, PartialEq)]
pub struct LargestStats {
    /// The step whose context grew, counted from zero.
    pub step: u64,
    /// The tools called in the step before it, each with its target.
    pub tools: Vec<String>,
    /// Tokens the step added.
    pub growth: u64,
    /// Growth times the steps it stayed in the context.
    pub cost: i64,
}

#[derive(Serialize, JsonSchema, Clone, Debug, Default, PartialEq)]
pub struct SessionStats {
    /// What this entry is: `turn 1 (go)`, `subagent ses_…` or `session ses_…`.
    pub label: String,
    /// Zero for a turn or the session asked for, then one per subagent level.
    pub depth: u32,
    pub steps: u64,
    /// The sum of every step's context.
    pub resent: u64,
    /// The sum of input tokens not served from the cache.
    pub uncached: u64,
    /// Cache reads as a share of input plus cache reads; zero when both are zero.
    pub cache_share: f64,
    pub output: u64,
    pub reasoning: u64,
    /// The middle context, sorted; zero without per-step numbers.
    pub median: u64,
    /// The ninth decile of the contexts; zero without per-step numbers.
    pub p90: u64,
    pub peak: u64,
    pub compactions: u64,
    /// What filled the context, sorted by cost descending.
    pub kinds: Vec<KindStats>,
    /// The ten steps that added most to the context, largest first.
    pub largest: Vec<LargestStats>,
    /// Anything this entry could not read, in words.
    pub note: Option<String>,
}

#[derive(Serialize, JsonSchema, Clone, Debug, Default, PartialEq)]
pub struct StatsTotals {
    pub steps: u64,
    pub resent: u64,
    pub uncached: u64,
    /// Cache reads as a share of input plus cache reads.
    pub cache_share: f64,
    pub output: u64,
    pub reasoning: u64,
    pub compactions: u64,
    /// The largest context of all the sessions.
    pub peak: u64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Numbers {
    pub steps: u64,
    pub input: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub resent: u64,
    pub output: u64,
    pub reasoning: u64,
    pub median: u64,
    pub p90: u64,
    pub peak: u64,
    pub compactions: u64,
    pub kinds: Vec<KindStats>,
    pub largest: Vec<LargestStats>,
}

impl Numbers {
    fn cache_share(&self) -> f64 {
        let total = self.input.saturating_add(self.cache_read);
        if total == 0 { 0.0 } else { self.cache_read as f64 / total as f64 }
    }

    fn session(&self, label: String, depth: u32, note: Option<String>) -> SessionStats {
        SessionStats {
            label,
            depth,
            steps: self.steps,
            resent: self.resent,
            uncached: self.input,
            cache_share: self.cache_share(),
            output: self.output,
            reasoning: self.reasoning,
            median: self.median,
            p90: self.p90,
            peak: self.peak,
            compactions: self.compactions,
            kinds: self.kinds.clone(),
            largest: self.largest.clone(),
            note,
        }
    }
}

/// Context, resent, deciles, compactions, and what filled it, from parsed data.
pub fn numbers(steps: &[Step], calls: &[Vec<Call>], marked: &[usize]) -> Numbers {
    let context: Vec<u64> = steps.iter().map(|step| step.context).collect();
    let count = context.len();
    let mut out = Numbers { steps: count as u64, ..Default::default() };
    if count == 0 {
        return out;
    }
    out.input = steps.iter().fold(0, |sum, step| sum.saturating_add(step.input));
    out.cache_read = steps.iter().fold(0, |sum, step| sum.saturating_add(step.cache_read));
    out.output = steps.iter().fold(0, |sum, step| sum.saturating_add(step.output));
    out.reasoning = steps.iter().fold(0, |sum, step| sum.saturating_add(step.reasoning));
    out.resent = context.iter().fold(0, |sum, value| sum.saturating_add(*value));
    let mut sorted = context.clone();
    sorted.sort_unstable();
    out.median = sorted[count / 2];
    out.p90 = sorted[(count * 9 / 10).min(count - 1)];
    out.peak = *sorted.last().unwrap_or(&0);

    let marked: BTreeSet<usize> = marked.iter().copied().filter(|at| *at < count).collect();
    let compaction: Vec<bool> = (0..count)
        .map(|at| {
            marked.contains(&at)
                || (at > 0 && context[at - 1] > 50_000 && (context[at] as u128) < (context[at - 1] as u128) * 7 / 10)
        })
        .collect();
    out.compactions = compaction.iter().filter(|is| **is).count() as u64;

    let mut starts = vec![0usize];
    starts.extend((1..count).filter(|at| compaction[*at]));
    let mut charged: BTreeMap<String, (i64, i64)> = BTreeMap::new();
    for (index, start) in starts.iter().copied().enumerate() {
        let end = starts.get(index + 1).map_or(count - 1, |next| next - 1);
        let span = end - start + 1;
        let growth = context[start] as i64;
        let kind = if start == 0 { "start" } else { "compaction" };
        let entry = charged.entry(kind.into()).or_default();
        entry.0 = entry.0.saturating_add(growth);
        entry.1 = entry.1.saturating_add(growth.saturating_mul(span as i64));
        for at in start + 1..=end {
            if compaction[at] {
                continue;
            }
            let growth = context[at] as i64 - context[at - 1] as i64;
            let tools = calls.get(at - 1).map(Vec::as_slice).unwrap_or(&[]);
            let cost = growth.saturating_mul((end - at + 1) as i64);
            if tools.is_empty() {
                let entry = charged.entry("reply".into()).or_default();
                entry.0 = entry.0.saturating_add(growth);
                entry.1 = entry.1.saturating_add(cost);
            } else {
                let share = growth / tools.len() as i64;
                let remainder = growth - share * tools.len() as i64;
                for (index, call) in tools.iter().enumerate() {
                    let part = share + if index == 0 { remainder } else { 0 };
                    let entry = charged.entry(call.name.clone()).or_default();
                    entry.0 = entry.0.saturating_add(part);
                    entry.1 = entry.1.saturating_add(part.saturating_mul((end - at + 1) as i64));
                }
            }
            if growth > 0 {
                out.largest.push(LargestStats {
                    step: at as u64,
                    tools: tools.iter().map(display).collect(),
                    growth: growth as u64,
                    cost,
                });
            }
        }
    }
    out.kinds = charged.into_iter().map(|(kind, (growth, cost))| KindStats { kind, growth, cost }).collect();
    out.kinds.sort_by(|a, b| b.cost.cmp(&a.cost).then_with(|| a.kind.cmp(&b.kind)));
    out.largest.sort_by(|a, b| b.growth.cmp(&a.growth).then_with(|| a.step.cmp(&b.step)));
    out.largest.truncate(10);
    out
}

fn display(call: &Call) -> String {
    match call.target.as_deref().filter(|target| !target.is_empty()) {
        Some(target) => format!("{} {target}", call.name),
        None => call.name.clone(),
    }
}

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
    }
}

/// The brief's target rule for an OpenCode tool: filePath, then command, then pattern.
fn target_of(input: Option<&serde_json::Map<String, Value>>) -> Option<String> {
    target_in(input, &["filePath", "command", "pattern"])
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
pub fn parse_opencode(lines: impl IntoIterator<Item = String>) -> Parsed {
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
pub fn parse_export(text: &str) -> Result<Parsed, String> {
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
pub fn parse_claude(lines: impl IntoIterator<Item = String>) -> Parsed {
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
        });
        out.calls.push(calls);
    }
    note_parse_errors(&mut out);
    out
}

/// Command Code's stream: each finished model request is one step, and the
/// tools it queued belong to it.
pub fn parse_commandcode(lines: impl IntoIterator<Item = String>) -> Parsed {
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
pub fn parse_commandcode_transcript(lines: impl IntoIterator<Item = String>) -> Parsed {
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

fn is_session_id(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 36
        && bytes.iter().enumerate().all(|(at, byte)| match at {
            8 | 13 | 18 | 23 => *byte == b'-',
            _ => byte.is_ascii_hexdigit(),
        })
}

fn recorded(usage: &Usage) -> Numbers {
    let context = usage.input.saturating_add(usage.cache_read).saturating_add(usage.cache_write);
    Numbers {
        steps: usage.steps,
        input: usage.input,
        cache_read: usage.cache_read,
        cache_write: usage.cache_write,
        resent: context,
        output: usage.output,
        reasoning: usage.reasoning,
        median: 0,
        p90: 0,
        peak: usage.largest_request,
        compactions: 0,
        kinds: Vec::new(),
        largest: Vec::new(),
    }
}

pub fn totals(numbers: &[Numbers]) -> StatsTotals {
    let mut out = StatsTotals::default();
    let mut cache_read = 0u64;
    let mut input = 0u64;
    for entry in numbers {
        out.steps = out.steps.saturating_add(entry.steps);
        out.resent = out.resent.saturating_add(entry.resent);
        out.uncached = out.uncached.saturating_add(entry.input);
        out.output = out.output.saturating_add(entry.output);
        out.reasoning = out.reasoning.saturating_add(entry.reasoning);
        out.compactions = out.compactions.saturating_add(entry.compactions);
        out.peak = out.peak.max(entry.peak);
        cache_read = cache_read.saturating_add(entry.cache_read);
        input = input.saturating_add(entry.input);
    }
    let cached = input.saturating_add(cache_read);
    out.cache_share = if cached == 0 { 0.0 } else { cache_read as f64 / cached as f64 };
    out
}

/// The jobs a brief names: lines that start `## Job ` or `### Job `; one when none.
pub fn jobs_in(text: &str) -> u64 {
    let count = text.lines().filter(|line| line.starts_with("## Job ") || line.starts_with("### Job ")).count();
    u64::try_from(count).unwrap_or(u64::MAX).max(1)
}

/// Additions plus deletions in a patch, without its `+++` and `---` headers.
pub fn patch_lines(text: &str) -> u64 {
    let count = text
        .lines()
        .filter(|line| {
            (line.starts_with('+') && !line.starts_with("+++")) || (line.starts_with('-') && !line.starts_with("---"))
        })
        .count();
    u64::try_from(count).unwrap_or(u64::MAX)
}

/// Additions plus deletions in `git diff --numstat` output; a binary file's `-` is zero.
pub fn numstat_lines(text: &str) -> u64 {
    let mut total = 0u64;
    for line in text.lines() {
        let mut fields = line.splitn(3, '\t');
        let Some(added) = fields.next() else { continue };
        let Some(removed) = fields.next() else { continue };
        if fields.next().is_none() {
            continue;
        }
        total = total.saturating_add(added.parse::<u64>().unwrap_or(0));
        total = total.saturating_add(removed.parse::<u64>().unwrap_or(0));
    }
    total
}

struct Entry {
    numbers: Numbers,
    label: String,
    depth: u32,
    note: Option<String>,
}

fn read_text(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn read_turn(cli: Cli, path: &Path, usage: &Usage) -> (Numbers, Option<String>, Vec<String>, Vec<String>) {
    let text = match read_text(path) {
        Ok(text) => text,
        Err(e) => {
            let words = format!("cannot read {}: {e}", path.display());
            return (recorded(usage), Some(format!("{words}; recorded totals only")), Vec::new(), vec![words]);
        }
    };
    let lines = text.lines().map(str::to_string);
    let parsed = match cli {
        Cli::Opencode => parse_opencode(lines),
        Cli::Claude => parse_claude(lines),
        Cli::CommandCode => parse_commandcode(lines),
        Cli::Codex => return (recorded(usage), Some(NO_STEPS.into()), Vec::new(), Vec::new()),
    };
    if parsed.steps.is_empty() {
        let note = match (parsed.note.clone(), usage.steps > 0) {
            (Some(note), true) => format!("{note}; recorded totals only"),
            (Some(note), false) => note,
            (None, true) => format!("no steps in {}; recorded totals only", path.display()),
            (None, false) => format!("no steps in {}", path.display()),
        };
        let numbers = if usage.steps > 0 { recorded(usage) } else { Numbers::default() };
        return (numbers, Some(note), parsed.children, Vec::new());
    }
    (numbers(&parsed.steps, &parsed.calls, &[]), parsed.note, parsed.children, Vec::new())
}

fn export_session(config: &Config, folder: &Path, session: &str) -> Result<String, String> {
    let prefix = worker::opencode_prefix(config).map_err(|e| e.message)?;
    let mut cmd = Command::new(&prefix[0]);
    cmd.args(&prefix[1..])
        .args(["session", "export", "--standalone", session])
        .current_dir(folder)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    crate::task::runner::prepare_child(&mut cmd, None);
    // No `no_console`: like the worker, it shares this terminal's hidden console.
    let mut child = cmd.spawn().map_err(|e| format!("cannot run the export: {e}"))?;
    let mut stdout = child.stdout.take().expect("piped stdout");
    let mut stderr = child.stderr.take().expect("piped stderr");
    let out_thread = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes);
        bytes
    });
    let err_thread = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stderr.read_to_end(&mut bytes);
        bytes
    });
    let deadline = Instant::now() + Duration::from_secs(60);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("export timed out".into());
            }
        }
    };
    let out = out_thread.join().unwrap_or_default();
    let err = err_thread.join().unwrap_or_default();
    if !status.success() {
        let text = String::from_utf8_lossy(&err);
        let text = text.trim();
        return Err(if text.is_empty() {
            format!("export exited with code {}", status.code().unwrap_or(-1))
        } else {
            cut(text, 400)
        });
    }
    Ok(String::from_utf8_lossy(&out).into_owned())
}

enum Child {
    Opencode(String),
    CommandCode(PathBuf),
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

fn one_candidate(candidates: &mut Vec<PathBuf>) -> Option<PathBuf> {
    candidates.sort();
    candidates.dedup();
    if candidates.len() == 1 { candidates.pop() } else { None }
}

fn cc_projects_root() -> PathBuf {
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

fn transcript_file(name: &str) -> bool {
    name.ends_with(".jsonl") && !name.contains(".checkpoints.") && !name.contains(".prompts.")
}

fn commandcode_transcript(value: &str, folder: &Path) -> Result<PathBuf, RpcError> {
    let missing = |words: &str| {
        RpcError::new(
            "not_found",
            format!("no Command Code transcript for {value:?}: {words}"),
            "give a session id or the path to its .jsonl",
        )
    };
    if value.ends_with(".jsonl") {
        let path = PathBuf::from(value);
        let path = if path.is_absolute() { path } else { folder.join(path) };
        return path.is_file().then_some(path).ok_or_else(|| missing("the file is not there"));
    }
    let mut found = Vec::new();
    for entry in std::fs::read_dir(cc_projects_root()).into_iter().flatten().flatten() {
        let dir = entry.path();
        let path = dir.join(format!("{value}.jsonl"));
        if dir.is_dir() && path.is_file() {
            found.push(path);
        }
    }
    let empty = found.is_empty();
    one_candidate(&mut found)
        .ok_or_else(|| missing(if empty { "no project has it" } else { "more than one project has it" }))
}

fn find_agent_transcript(agent: &AgentCall, transcript: &Path) -> Option<PathBuf> {
    let sibling = transcript.parent()?;
    let mut found = Vec::new();
    for candidate in &agent.candidates {
        let path = PathBuf::from(candidate);
        if path.extension().is_some_and(|extension| extension == "jsonl") {
            let path = if path.is_absolute() { path } else { sibling.join(path) };
            let stem = path.file_stem().and_then(|stem| stem.to_str()).unwrap_or_default();
            let looks_like_transcript = is_session_id(stem) || path.to_string_lossy().contains(".commandcode");
            if looks_like_transcript && path.is_file() && path != transcript {
                found.push(path);
            }
        } else {
            let path = sibling.join(format!("{candidate}.jsonl"));
            if path.is_file() && path != transcript {
                found.push(path);
            }
            for entry in std::fs::read_dir(cc_projects_root()).into_iter().flatten().flatten() {
                let path = entry.path().join(format!("{candidate}.jsonl"));
                if path.is_file() && path != transcript {
                    found.push(path);
                }
            }
        }
    }
    if found.is_empty() {
        for entry in std::fs::read_dir(sibling).into_iter().flatten().flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let stem = name.strip_suffix(".jsonl").unwrap_or_default();
            if transcript_file(&name) && (stem == agent.id || stem.contains(&agent.id)) {
                let path = entry.path();
                if path != transcript {
                    found.push(path);
                }
            }
        }
    }
    one_candidate(&mut found)
}

fn commandcode_children(
    parsed: &Parsed,
    transcript: &Path,
    depth: u32,
    queue: &mut VecDeque<(Child, u32)>,
    entries: &mut Vec<Entry>,
) {
    if depth >= 3 {
        return;
    }
    for agent in &parsed.agents {
        match find_agent_transcript(agent, transcript) {
            Some(path) => queue.push_back((Child::CommandCode(path), depth + 1)),
            None => entries.push(Entry {
                numbers: Numbers::default(),
                label: format!("subagent {}", agent.id),
                depth: depth + 1,
                note: Some("transcript not found".into()),
            }),
        }
    }
}

fn read_children(
    mut queue: VecDeque<(Child, u32)>,
    entries: &mut Vec<Entry>,
    problems: &mut Vec<String>,
    config: &Config,
    folder: &Path,
    visited: &mut BTreeSet<String>,
) {
    while let Some((child, depth)) = queue.pop_front() {
        match child {
            Child::Opencode(id) => {
                if !visited.insert(id.clone()) {
                    continue;
                }
                let label = format!("subagent {id}");
                match export_session(config, folder, &id) {
                    Ok(text) => match parse_export(&text) {
                        Ok(parsed) => {
                            if depth < 3 {
                                for child in parsed.children {
                                    queue.push_back((Child::Opencode(child), depth + 1));
                                }
                            }
                            entries.push(Entry {
                                numbers: numbers(&parsed.steps, &parsed.calls, &[]),
                                label,
                                depth,
                                note: parsed.note,
                            });
                        }
                        Err(e) => {
                            let words = format!("cannot read the export of {id}: {e}");
                            problems.push(words.clone());
                            entries.push(Entry { numbers: Numbers::default(), label, depth, note: Some(words) });
                        }
                    },
                    Err(e) => {
                        let words = format!("cannot export {id}: {e}");
                        problems.push(words.clone());
                        entries.push(Entry { numbers: Numbers::default(), label, depth, note: Some(words) });
                    }
                }
            }
            Child::CommandCode(path) => {
                if !visited.insert(path.display().to_string()) {
                    continue;
                }
                let label =
                    format!("subagent {}", path.file_stem().and_then(|stem| stem.to_str()).unwrap_or("transcript"));
                match read_text(&path) {
                    Ok(text) => {
                        let parsed = parse_commandcode_transcript(text.lines().map(str::to_string));
                        commandcode_children(&parsed, &path, depth, &mut queue, entries);
                        entries.push(Entry {
                            numbers: numbers(&parsed.steps, &parsed.calls, &parsed.marked),
                            label,
                            depth,
                            note: parsed.note,
                        });
                    }
                    Err(e) => {
                        let words = format!("cannot read {}: {e}", path.display());
                        problems.push(words.clone());
                        entries.push(Entry { numbers: Numbers::default(), label, depth, note: Some(words) });
                    }
                }
            }
        }
    }
}

fn jobs_of(folder: &Path, problems: &mut Vec<String>) -> u64 {
    let path = folder.join("brief.md");
    match read_text(&path) {
        Ok(text) => jobs_in(&text),
        Err(_) if !path.exists() => 1,
        Err(e) => {
            problems.push(format!("cannot read {}: {e}", path.display()));
            1
        }
    }
}

fn git_numstat(worktree: &str, base: &str) -> Result<u64, String> {
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(worktree)
        .args(["-c", "core.quotePath=false", "-c", "color.ui=false"])
        .args(["diff", "--numstat", "--no-renames", base])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    crate::platform::no_console(&mut cmd);
    let out = cmd.output().map_err(|e| format!("cannot run git: {e}"))?;
    if !out.status.success() {
        let text = String::from_utf8_lossy(&out.stderr);
        let text = text.trim();
        return Err(if text.is_empty() {
            format!("git exited with code {}", out.status.code().unwrap_or(-1))
        } else {
            cut(text, 400)
        });
    }
    Ok(numstat_lines(&String::from_utf8_lossy(&out.stdout)))
}

fn changed_lines(state: &Path, task: &crate::task::types::Task, problems: &mut Vec<String>) -> Option<u64> {
    let patch = state.join("patches").join(format!("{}.patch", task.name));
    match read_text(&patch) {
        Ok(text) => return Some(patch_lines(&text)),
        Err(_) if !patch.exists() => {}
        Err(e) => problems.push(format!("cannot read {}: {e}", patch.display())),
    }
    let worktree = task.worktree.as_deref().filter(|path| Path::new(path).is_dir())?;
    match git_numstat(worktree, &task.base) {
        Ok(lines) => Some(lines),
        Err(e) => {
            problems.push(format!("cannot count changed lines in {worktree}: {e}"));
            None
        }
    }
}

fn task_result(
    state: &Path,
    config: &Config,
    task: &crate::task::types::Task,
    workers: &[worker::Worker],
) -> TaskStatsResult {
    let folder = crate::task::task_folder(state, &task.name);
    let mut entries = Vec::new();
    let mut problems = Vec::new();
    let mut queue = VecDeque::new();
    let mut visited = BTreeSet::new();
    for turn in &task.turns {
        if let Some(session) = &turn.session {
            visited.insert(session.clone());
        }
        let label = format!("turn {} ({})", turn.n, turn.worker);
        let cli = turn.cli_or_lookup(workers);
        let path = folder.join(format!("turn-{}.jsonl", turn.n));
        let (numbers, note, children, mut trouble) = read_turn(cli, &path, &turn.usage);
        problems.append(&mut trouble);
        for child in children {
            queue.push_back((Child::Opencode(child), 1));
        }
        entries.push(Entry { numbers, label, depth: 0, note });
    }
    read_children(queue, &mut entries, &mut problems, config, Path::new(&task.repo), &mut visited);
    let jobs = jobs_of(&folder, &mut problems);
    let changed = changed_lines(state, task, &mut problems);
    finish(task.name.clone(), entries, problems, Some(jobs), changed)
}

fn session_result(config: &Config, folder: &Path, session: &str) -> Result<TaskStatsResult, RpcError> {
    let text = export_session(config, folder, session).map_err(|e| {
        RpcError::new(
            "unavailable",
            format!("cannot export session {session}: {e}"),
            "check that OpenCode can run in the folder",
        )
    })?;
    let parsed = parse_export(&text).map_err(|e| {
        RpcError::new("unavailable", format!("cannot read the export of {session}: {e}"), "try the export again")
    })?;
    let numbers = numbers(&parsed.steps, &parsed.calls, &[]);
    let mut entries = vec![Entry { numbers, label: format!("session {session}"), depth: 0, note: parsed.note }];
    let mut problems = Vec::new();
    let mut queue = VecDeque::new();
    for child in parsed.children {
        queue.push_back((Child::Opencode(child), 1));
    }
    let mut visited = BTreeSet::from([session.to_string()]);
    read_children(queue, &mut entries, &mut problems, config, folder, &mut visited);
    Ok(finish(session.to_string(), entries, problems, None, None))
}

fn commandcode_result(value: &str, folder: &Path, config: &Config) -> Result<TaskStatsResult, RpcError> {
    let path = commandcode_transcript(value, folder)?;
    let subject =
        path.file_stem().and_then(|stem| stem.to_str()).filter(|stem| !stem.is_empty()).unwrap_or(value).to_string();
    let text = read_text(&path).map_err(|e| {
        RpcError::new(
            "unavailable",
            format!("cannot read {}: {e}", path.display()),
            "check the transcript's permissions",
        )
    })?;
    let parsed = parse_commandcode_transcript(text.lines().map(str::to_string));
    let numbers = numbers(&parsed.steps, &parsed.calls, &parsed.marked);
    let mut entries = vec![Entry { numbers, label: format!("session {subject}"), depth: 0, note: parsed.note.clone() }];
    let mut problems = Vec::new();
    let mut queue = VecDeque::new();
    commandcode_children(&parsed, &path, 0, &mut queue, &mut entries);
    let mut visited = BTreeSet::from([path.display().to_string()]);
    read_children(queue, &mut entries, &mut problems, config, folder, &mut visited);
    Ok(finish(subject, entries, problems, None, None))
}

fn finish(
    subject: String,
    entries: Vec<Entry>,
    problems: Vec<String>,
    jobs: Option<u64>,
    changed_lines: Option<u64>,
) -> TaskStatsResult {
    let raw: Vec<Numbers> = entries.iter().map(|entry| entry.numbers.clone()).collect();
    let totals = totals(&raw);
    let sessions: Vec<SessionStats> = entries
        .iter()
        .map(|entry| entry.numbers.session(entry.label.clone(), entry.depth, entry.note.clone()))
        .collect();
    let resent_per_job = jobs.filter(|jobs| *jobs > 0).filter(|_| totals.resent > 0).map(|jobs| totals.resent / jobs);
    let resent_per_changed_line =
        changed_lines.filter(|lines| *lines > 0).filter(|_| totals.resent > 0).map(|lines| totals.resent / lines);
    let table = table(&subject, &sessions, &totals, jobs, changed_lines);
    TaskStatsResult {
        subject,
        sessions,
        totals,
        jobs,
        changed_lines,
        resent_per_job,
        resent_per_changed_line,
        problems,
        table,
    }
}

/// The Markdown table `kom task stats` prints.
pub fn table(
    subject: &str,
    sessions: &[SessionStats],
    totals: &StatsTotals,
    jobs: Option<u64>,
    changed_lines: Option<u64>,
) -> String {
    let mut lines = vec![format!(
        "**{}** · {} steps · {} resent · {:.1}% cached · peak {} · {}",
        cell(subject),
        totals.steps,
        token_text(totals.resent),
        totals.cache_share * 100.0,
        token_text(totals.peak),
        count(totals.compactions, "compaction", "compactions"),
    )];
    lines.push(String::new());
    lines.push("| Session | Depth | Steps | Resent | Cache | Peak | Compactions | Note |".into());
    lines.push("| --- | --- | --- | --- | --- | --- | --- | --- |".into());
    for session in sessions {
        lines.push(format!(
            "| {} | {} | {} | {} | {:.1}% | {} | {} | {} |",
            cell(&session.label),
            session.depth,
            session.steps,
            token_text(session.resent),
            session.cache_share * 100.0,
            token_text(session.peak),
            session.compactions,
            cell(session.note.as_deref().unwrap_or_default()),
        ));
    }

    lines.push(String::new());
    lines.push("### What filled the context".into());
    lines.push(String::new());
    let kinds = total_kinds(sessions);
    if kinds.is_empty() {
        lines.push("No step-by-step context for these sessions.".into());
    } else {
        lines.push("| Kind | Growth | Cost | Share |".into());
        lines.push("| --- | --- | --- | --- |".into());
        for kind in kinds {
            let share = if totals.resent == 0 { 0.0 } else { kind.cost as f64 * 100.0 / totals.resent as f64 };
            lines.push(format!(
                "| {} | {} | {} | {:.1}% |",
                cell(&kind.kind),
                signed_text(kind.growth),
                signed_text(kind.cost),
                share
            ));
        }
    }

    lines.push(String::new());
    lines.push("### Largest additions".into());
    lines.push(String::new());
    let mut largest: Vec<(&str, &LargestStats)> = sessions
        .iter()
        .flat_map(|session| session.largest.iter().map(move |item| (session.label.as_str(), item)))
        .collect();
    largest
        .sort_by(|a, b| b.1.growth.cmp(&a.1.growth).then_with(|| a.1.step.cmp(&b.1.step)).then_with(|| a.0.cmp(b.0)));
    largest.truncate(10);
    if largest.is_empty() {
        lines.push("No step-by-step context for these sessions.".into());
    } else {
        lines.push("| Step | Tools and targets | Growth | Cost |".into());
        lines.push("| --- | --- | --- | --- |".into());
        for (label, item) in largest {
            lines.push(format!(
                "| {} step {} | {} | {} | {} |",
                cell(label),
                item.step,
                cell(&item.tools.join(", ")),
                token_text(item.growth),
                signed_text(item.cost)
            ));
        }
    }

    if let Some(jobs) = jobs {
        let mut words = format!("Per unit of work: {}", count(jobs, "job", "jobs"));
        if let Some(changed) = changed_lines {
            words.push_str(&format!(", {}", count(changed, "changed line", "changed lines")));
        }
        if let Some(per_job) = (jobs > 0 && totals.resent > 0).then(|| totals.resent / jobs) {
            words.push_str(&format!("; {} resent per job", token_text(per_job)));
            if let Some(changed) = changed_lines.filter(|lines| *lines > 0) {
                words.push_str(&format!(", {} per changed line", token_text(totals.resent / changed)));
            }
        }
        lines.push(String::new());
        lines.push(format!("{words}."));
    }
    lines.join("\n")
}

/// The kinds of every session added up, sorted by cost descending.
pub fn total_kinds(sessions: &[SessionStats]) -> Vec<KindStats> {
    let mut kinds: BTreeMap<String, (i64, i64)> = BTreeMap::new();
    for session in sessions {
        for kind in &session.kinds {
            let entry = kinds.entry(kind.kind.clone()).or_default();
            entry.0 = entry.0.saturating_add(kind.growth);
            entry.1 = entry.1.saturating_add(kind.cost);
        }
    }
    let mut out: Vec<KindStats> =
        kinds.into_iter().map(|(kind, (growth, cost))| KindStats { kind, growth, cost }).collect();
    out.sort_by(|a, b| b.cost.cmp(&a.cost).then_with(|| a.kind.cmp(&b.kind)));
    out
}

fn cell(text: &str) -> String {
    text.replace('|', "/").replace(['\r', '\n'], " ")
}

fn count(n: u64, one: &str, many: &str) -> String {
    if n == 1 { format!("{n} {one}") } else { format!("{n} {many}") }
}

fn token_text(value: u64) -> String {
    if value >= 1_000_000 {
        let text = format!("{:.1}", value as f64 / 1_000_000.0);
        format!("{}M", text.trim_end_matches('0').trim_end_matches('.'))
    } else {
        format!("{}K", (value as f64 / 1000.0).round() as u64)
    }
}

fn signed_text(value: i64) -> String {
    if value < 0 { format!("-{}", token_text(value.unsigned_abs())) } else { token_text(value as u64) }
}

#[derive(Debug)]
enum Choice {
    Task(String),
    Opencode(String),
    CommandCode(String),
}

fn choice(p: &TaskStatsParams) -> Result<Choice, RpcError> {
    match (p.task.as_deref(), p.opencode.as_deref(), p.commandcode.as_deref()) {
        (Some(task), None, None) => Ok(Choice::Task(task.to_string())),
        (None, Some(session), None) => Ok(Choice::Opencode(session.to_string())),
        (None, None, Some(session)) => Ok(Choice::CommandCode(session.to_string())),
        _ => Err(RpcError::invalid_params("give exactly one of `task`, `opencode` and `commandcode`")),
    }
}

fn absolute(folder: &str) -> PathBuf {
    let path = PathBuf::from(folder);
    if path.is_absolute() {
        return path;
    }
    std::env::current_dir().unwrap_or_default().join(path)
}

pub async fn stats(core: &Arc<Core>, p: TaskStatsParams) -> Result<TaskStatsResult, RpcError> {
    let subject = choice(&p)?;
    let config = crate::config::load(&core.paths.config).unwrap_or_default();
    match subject {
        Choice::Task(name) => {
            let task = core.tasks.snapshot(&name).ok_or_else(|| crate::task::not_found(&name))?;
            let state = core.paths.state.clone();
            let workers = crate::task::workers::all(&core.tasks.workers());
            tokio::task::spawn_blocking(move || task_result(&state, &config, &task, &workers))
                .await
                .map_err(|e| RpcError::new("unavailable", format!("cannot read the task's numbers: {e}"), "try again"))
        }
        Choice::Opencode(session) => {
            let folder = absolute(p.folder.as_deref().unwrap_or("."));
            tokio::task::spawn_blocking(move || session_result(&config, &folder, &session)).await.map_err(|e| {
                RpcError::new("unavailable", format!("cannot read the session's numbers: {e}"), "try again")
            })?
        }
        Choice::CommandCode(session) => {
            let folder = absolute(p.folder.as_deref().unwrap_or("."));
            tokio::task::spawn_blocking(move || commandcode_result(&session, &folder, &config)).await.map_err(|e| {
                RpcError::new("unavailable", format!("cannot read the session's numbers: {e}"), "try again")
            })?
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn call(name: &str, target: Option<&str>) -> Call {
        Call { name: name.into(), target: target.map(String::from) }
    }

    fn steps(contexts: &[u64]) -> Vec<Step> {
        contexts.iter().map(|context| Step { context: *context, input: *context, ..Default::default() }).collect()
    }

    fn kind<'a>(numbers: &'a Numbers, name: &str) -> &'a KindStats {
        numbers
            .kinds
            .iter()
            .find(|kind| kind.kind == name)
            .unwrap_or_else(|| panic!("no {name} in {:?}", numbers.kinds))
    }

    #[test]
    fn kinds_add_up_to_resent() {
        let steps = steps(&[1000, 1500, 1500, 4000, 4200]);
        let calls = vec![
            vec![call("read", Some("a.rs"))],
            vec![call("edit", None)],
            vec![call("shell", None)],
            Vec::new(),
            vec![call("edit", None)],
        ];
        let numbers = numbers(&steps, &calls, &[]);
        assert_eq!(numbers.resent, 12_200);
        assert_eq!(kind(&numbers, "start").cost, 5_000);
        assert_eq!((kind(&numbers, "read").growth, kind(&numbers, "read").cost), (500, 2_000));
        assert_eq!((kind(&numbers, "edit").growth, kind(&numbers, "edit").cost), (0, 0));
        assert_eq!((kind(&numbers, "shell").growth, kind(&numbers, "shell").cost), (2_500, 5_000));
        assert_eq!((kind(&numbers, "reply").growth, kind(&numbers, "reply").cost), (200, 200));
        assert_eq!(numbers.kinds.iter().map(|kind| kind.cost).sum::<i64>(), 12_200);
        assert_eq!(numbers.largest[0].step, 3);
        assert!(numbers.largest[0].tools.iter().any(|tool| tool.starts_with("shell")), "{:?}", numbers.largest);
    }

    #[test]
    fn a_compaction_starts_a_stretch() {
        let steps = steps(&[100_000, 300_000, 90_000, 120_000]);
        let calls = vec![vec![call("read", Some("a.rs"))], Vec::new(), vec![call("edit", None)], Vec::new()];
        let numbers = numbers(&steps, &calls, &[]);
        assert_eq!(numbers.compactions, 1);
        assert_eq!(kind(&numbers, "start").cost, 200_000);
        assert_eq!(kind(&numbers, "read").cost, 200_000);
        assert_eq!(kind(&numbers, "compaction").cost, 180_000);
        assert_eq!((kind(&numbers, "edit").growth, kind(&numbers, "edit").cost), (30_000, 30_000));
        assert_eq!(numbers.kinds.iter().map(|kind| kind.cost).sum::<i64>(), 610_000);
        assert_eq!((numbers.median, numbers.p90, numbers.peak), (120_000, 300_000, 300_000));
    }

    #[test]
    fn two_tools_in_one_step_split_the_growth() {
        let steps = steps(&[100, 301]);
        let calls = vec![vec![call("read", Some("a.rs")), call("grep", Some("b"))], Vec::new()];
        let numbers = numbers(&steps, &calls, &[]);
        assert_eq!((kind(&numbers, "read").growth, kind(&numbers, "grep").growth), (101, 100));
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
    fn a_brief_counts_jobs_and_a_patch_counts_its_lines() {
        assert_eq!(jobs_in("## Job 1.\n### Job 2.\n## Jobs\n"), 2);
        assert_eq!(jobs_in("nothing here"), 1);
        let patch = "--- a/a.rs\n+++ b/a.rs\n@@ -1,2 +1,3 @@\n context\n+one\n+two\n-three\n+four\n";
        assert_eq!(patch_lines(patch), 4);
        assert_eq!(
            numstat_lines("3\t1\tsrc/a.rs\n-\t-\timage.png\n2\t0\tb.rs\n"),
            6,
            "a binary file's dashes count as zero"
        );
    }

    fn params(task: Option<&str>, opencode: Option<&str>, commandcode: Option<&str>) -> TaskStatsParams {
        TaskStatsParams {
            task: task.map(String::from),
            opencode: opencode.map(String::from),
            commandcode: commandcode.map(String::from),
            folder: None,
        }
    }

    #[test]
    fn params_need_exactly_one_subject() {
        for (task, opencode, commandcode) in [
            (None, None, None),
            (Some("t"), Some("o"), None),
            (Some("t"), None, Some("c")),
            (None, Some("o"), Some("c")),
            (Some("t"), Some("o"), Some("c")),
        ] {
            assert_eq!(choice(&params(task, opencode, commandcode)).unwrap_err().kind, "invalid_params");
        }
        assert!(matches!(choice(&params(Some("t"), None, None)), Ok(Choice::Task(task)) if task == "t"));
        assert!(matches!(choice(&params(None, Some("o"), None)), Ok(Choice::Opencode(id)) if id == "o"));
        assert!(matches!(choice(&params(None, None, Some("c"))), Ok(Choice::CommandCode(id)) if id == "c"));
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
