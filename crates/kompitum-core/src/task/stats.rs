//! `kom task stats`: where a run's tokens went, step by step.

mod parse;
mod read;

pub use parse::project_folder;

use crate::api::{TaskStatsParams, TaskStatsResult};
use crate::config::Config;
use crate::rpc::RpcError;
use crate::runtime::Core;
use crate::task::types::{ChildUsage, Usage};
use read::{commandcode_result, session_result, task_result};
use schemars::JsonSchema;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// What a CLI without per-step numbers says instead.
const NO_STEPS: &str = "per-step numbers: not in this CLI's stream";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Step {
    pub context: u64,
    pub input: u64,
    pub output: u64,
    pub reasoning: u64,
    pub cache_read: u64,
    pub cache_write: u64,
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

/// One child session's steps as a `Usage`: its tokens, steps and largest request.
pub fn usage_of_steps(steps: &[Step]) -> Usage {
    let mut usage = Usage::default();
    for step in steps {
        usage.steps = usage.steps.saturating_add(1);
        usage.input = usage.input.saturating_add(step.input);
        usage.output = usage.output.saturating_add(step.output);
        usage.reasoning = usage.reasoning.saturating_add(step.reasoning);
        usage.cache_read = usage.cache_read.saturating_add(step.cache_read);
        usage.cache_write = usage.cache_write.saturating_add(step.cache_write);
        usage.largest_request = usage.largest_request.max(step.context);
    }
    usage
}

/// The child sessions an OpenCode turn's stream names, each with its exported usage.
/// A child that can't be exported is a warning in words, never a silent skip.
pub(crate) fn children_usage(config: &Config, folder: &Path, path: &Path) -> (Vec<ChildUsage>, Vec<String>) {
    let Ok(bytes) = std::fs::read(path) else { return (Vec::new(), Vec::new()) };
    let parsed = parse::parse_opencode(String::from_utf8_lossy(&bytes).lines().map(str::to_string));
    let mut children: Vec<ChildUsage> = Vec::new();
    let mut warnings = Vec::new();
    for session in parsed.children {
        if children.iter().any(|child| child.session == session) {
            continue;
        }
        match read::export_session(config, folder, &session) {
            Ok(text) => match parse::parse_export(&text) {
                Ok(export) => children.push(ChildUsage { session, usage: usage_of_steps(&export.steps) }),
                Err(e) => warnings
                    .push(format!("subagent session {session} could not be read, so its usage is not counted: {e}")),
            },
            Err(e) => warnings
                .push(format!("subagent session {session} could not be exported, so its usage is not counted: {e}")),
        }
    }
    (children, warnings)
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
    fn a_childs_steps_sum_into_one_usage() {
        let steps = vec![
            Step { context: 10, input: 4, output: 2, reasoning: 1, cache_read: 3, cache_write: 3 },
            Step { context: 20, input: 8, output: 5, reasoning: 2, cache_read: 6, cache_write: 6 },
        ];
        let usage = usage_of_steps(&steps);
        assert_eq!(
            (usage.steps, usage.input, usage.output, usage.reasoning, usage.cache_read, usage.cache_write),
            (2, 12, 7, 3, 9, 9)
        );
        assert_eq!(usage.largest_request, 20, "the largest request, not the sum");
        assert_eq!(usage_of_steps(&[]), Usage::default());
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
}
