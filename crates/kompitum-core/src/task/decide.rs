//! What a task's record and the world mean for it: is a path in scope, may a
//! queued task start, is it time to cut a turn, and how did that turn end.
//! Pure: no IO, no processes, so every rule is a unit test.

use crate::task::opencode::RouteCheck;
use crate::task::quota::{self, Refusal};
use crate::task::types::{Beat, Ending, Task, TaskLine, TaskState};
use crate::task::worker::{Cli, TurnOutcome, Worker};

/// A `*` crosses `/`, as Python's `fnmatch` does: `src/*` admits `src/a/b.rs`.
fn glob_match(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();
    matches(&pattern, &text)
}

fn matches(pattern: &[char], text: &[char]) -> bool {
    let Some(&first) = pattern.first() else { return text.is_empty() };
    match first {
        '*' => {
            // Collapse runs of `*` so the recursion stays short.
            let mut rest = 1;
            while pattern.get(rest) == Some(&'*') {
                rest += 1;
            }
            (0..=text.len()).any(|at| matches(&pattern[rest..], &text[at..]))
        }
        '?' => !text.is_empty() && matches(&pattern[1..], &text[1..]),
        '[' => match class(pattern) {
            Some(class) => {
                let Some(&c) = text.first() else { return false };
                let inside = class.ranges.iter().any(|(from, to)| *from <= c && c <= *to);
                inside != class.negated && matches(&pattern[class.end..], &text[1..])
            }
            None => !text.is_empty() && text[0] == '[' && matches(&pattern[1..], &text[1..]),
        },
        c => !text.is_empty() && text[0] == c && matches(&pattern[1..], &text[1..]),
    }
}

/// `[abc]`, `[a-z]`, `[!0-9]`: its ranges, whether it is negated, and where it ends.
struct Class {
    ranges: Vec<(char, char)>,
    negated: bool,
    end: usize,
}

fn class(pattern: &[char]) -> Option<Class> {
    let mut i = 1;
    let negated = matches!(pattern.get(i), Some('!'));
    if negated {
        i += 1;
    }
    let start = i;
    let mut ranges = Vec::new();
    while i < pattern.len() {
        if pattern[i] == ']' && i > start {
            return Some(Class { ranges, negated, end: i + 1 });
        }
        let from = pattern[i];
        if pattern.get(i + 1) == Some(&'-')
            && let Some(&to) = pattern.get(i + 2)
            && to != ']'
        {
            ranges.push((from, to));
            i += 3;
            continue;
        }
        ranges.push((from, from));
        i += 1;
    }
    None
}

/// The parts of a glob before its first wildcard, for comparing two scopes.
pub fn static_prefix(glob: &str) -> Vec<String> {
    let normal = glob.replace('\\', "/");
    let mut parts = Vec::new();
    for part in normal.trim_matches('/').split('/') {
        if part.chars().any(|c| matches!(c, '*' | '?' | '[')) {
            break;
        }
        parts.push(part.to_string());
    }
    parts
}

/// Conservative: two scopes overlap when any pair of globs shares a static path prefix.
pub fn scopes_overlap(a: &[String], b: &[String]) -> bool {
    let all = vec!["**".to_string()];
    let (a, b) = (if a.is_empty() { &all } else { a }, if b.is_empty() { &all } else { b });
    for x in a {
        for y in b {
            let (px, py) = (static_prefix(x), static_prefix(y));
            let n = px.len().min(py.len());
            if px[..n] == py[..n] {
                return true;
            }
        }
    }
    false
}

/// `dir`, `dir/*` and `dir/**` all admit everything under `dir`; an empty
/// scope admits everything. Python's `fnmatchcase`, backslashes read as `/`.
pub fn in_scope(path: &str, scope: &[String]) -> bool {
    if scope.is_empty() {
        return true;
    }
    let path = path.replace('\\', "/");
    for glob in scope {
        let g = glob.replace('\\', "/").trim_matches('/').to_string();
        let below = format!("{}/", g.trim_end_matches('/'));
        if glob_match(&g, &path)
            || glob_match(&format!("{below}*"), &path)
            || (g.ends_with("/**") && path.starts_with(&format!("{}/", &g[..g.len() - 3])))
            || path.starts_with(&below)
        {
            return true;
        }
    }
    false
}

/// Why a queued task may not start yet, in the words of `waiting`.
pub fn may_start(task: &Task, others: &[&Task], cap: Option<usize>) -> Result<(), String> {
    let active = |task: &Task| matches!(task.state, TaskState::Running | TaskState::Checking);
    let running = others.iter().filter(|other| active(other)).count();
    if let Some(cap) = cap
        && running >= cap
    {
        return Err(format!("waiting for a task to finish: {running} of {cap} running"));
    }
    for other in others {
        if other.name == task.name || other.repo != task.repo {
            continue;
        }
        // An asking task builds nothing, so it holds no one-at-a-time slot, but its
        // worker goes on in the same files once answered, so it keeps its scope.
        if other.state == TaskState::Asking {
            if scopes_overlap(&task.scope, &other.scope) {
                return Err(format!("waiting for {}: it is asking, and its scope overlaps", other.name));
            }
            continue;
        }
        // A stranded task holds its work in a worktree too, so it goes first.
        let earlier =
            active(other) || (matches!(other.state, TaskState::Queued | TaskState::Stranded) && other.seq < task.seq);
        if !earlier {
            continue;
        }
        if task.one_at_a_time {
            return Err(if active(other) {
                format!("waiting for {}: this repository builds one task at a time", other.name)
            } else {
                format!("queued behind {}: this repository builds one task at a time", other.name)
            });
        }
        if scopes_overlap(&task.scope, &other.scope) {
            return Err(if active(other) {
                format!("waiting for {}: its scope overlaps", other.name)
            } else {
                format!("queued behind {}: its scope overlaps", other.name)
            });
        }
    }
    crate::task::workshop::may_start(task, others)
}

/// What the core last knew of a turn's terminal.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TerminalNow {
    pub live: bool,
    pub exit_code: Option<i32>,
    pub reason: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Wait,
    Kill(Ending),
    TurnOver,
}

/// Whether a running turn goes on, must be cut, or has ended.
pub fn step(task: &Task, terminal: Option<&TerminalNow>, beat: Option<&Beat>, now_ms: u64) -> Step {
    if !terminal.is_some_and(|terminal| terminal.live) {
        return Step::TurnOver;
    }
    // An end already decided, and the terminal still live: the kill was lost,
    // to a crash or a timeout. Kill again, or the worker runs with no limit.
    if let Some(ending) = task.ending {
        return Step::Kill(ending);
    }
    let Some(turn) = task.turns.last() else { return Step::TurnOver };
    if task.wall_s > 0 && now_ms.saturating_sub(turn.started_ms) > task.wall_s.saturating_mul(1000) {
        return Step::Kill(Ending::Wall);
    }
    let active = beat.map_or(turn.started_ms, |beat| beat.active_ms.max(turn.started_ms));
    if now_ms.saturating_sub(active) > task.idle_s.saturating_mul(1000) {
        return Step::Kill(Ending::Idle);
    }
    Step::Wait
}

/// How a turn ended: its state, the words for it, and a quota refusal when
/// the worker named one.
#[derive(Clone, Debug, PartialEq)]
pub struct TurnEnd {
    pub state: TaskState,
    pub error: Option<String>,
    pub refusal: Option<Refusal>,
}

impl TurnEnd {
    fn failed(message: impl Into<String>) -> TurnEnd {
        TurnEnd { state: TaskState::Failed, error: Some(message.into()), refusal: None }
    }
}

#[allow(clippy::too_many_arguments)]
/// The end of a turn, in the order `execute_turn` uses in the Python: an
/// explicit end first, then what the terminal said, then the stream's facts,
/// and at last the route the worker really used.
pub fn turn_end(
    task: &Task,
    workers: &[Worker],
    terminal: Option<&TerminalNow>,
    exit_file_code: Option<i32>,
    outcome: &TurnOutcome,
    route: Option<&RouteCheck>,
    stderr_tail: &str,
    now_ms: u64,
    offset_min: i32,
) -> TurnEnd {
    match task.ending {
        Some(Ending::Cancel) => return TurnEnd { state: TaskState::Cancelled, error: None, refusal: None },
        Some(Ending::Idle) => {
            let tool = outcome.tools.last().cloned().unwrap_or_else(|| "none".into());
            return TurnEnd {
                state: TaskState::TimedOut,
                error: Some(format!(
                    "no output, CPU or I/O for {} s (idle limit {} s), last tool: {tool}",
                    task.idle_s, task.idle_s
                )),
                refusal: None,
            };
        }
        Some(Ending::Wall) => {
            return TurnEnd {
                state: TaskState::TimedOut,
                error: Some(format!("ran past its {} s limit", task.wall_s)),
                refusal: None,
            };
        }
        None => {}
    }
    let code = terminal.and_then(|terminal| terminal.exit_code).or(exit_file_code);
    match terminal {
        None if code.is_none() => return TurnEnd::failed("the core stopped while it ran"),
        Some(terminal) if terminal.reason.as_deref().is_some_and(|reason| reason != "exited") => {
            let words = match terminal.reason.as_deref().unwrap_or_default() {
                "killed" => "its terminal was killed",
                "server_stop" | "stopped_by_restart" => "the core stopped while it ran",
                _ => "its terminal was lost",
            };
            return TurnEnd::failed(words);
        }
        Some(terminal) if !terminal.live && code.is_none() => {
            return TurnEnd::failed("its terminal was lost");
        }
        _ => {}
    }
    let turn = task.turns.last();
    let cli = turn.map_or(Cli::Opencode, |turn| turn.cli_or_lookup(workers));
    match cli {
        Cli::Codex => codex_end(task, code, outcome, now_ms, offset_min),
        Cli::Claude => crate::task::claude::end(task, code, outcome, now_ms, offset_min),
        Cli::CommandCode => {
            let key = turn.map(|turn| turn.worker.as_str()).unwrap_or_default();
            let route = workers.iter().find(|worker| worker.key == key).map_or("", |worker| worker.route.as_str());
            command_code_end(task, route, code, outcome, stderr_tail, now_ms, offset_min)
        }
        Cli::Opencode => opencode_end(task, code, outcome, route, stderr_tail, now_ms),
    }
}

pub(crate) fn session_end(task: &Task, outcome: &TurnOutcome) -> Option<TurnEnd> {
    let expected = task.turns.last().and_then(|turn| turn.resumes.clone());
    match &outcome.session {
        None => Some(TurnEnd::failed("worker did not report a session id")),
        Some(session) if expected.as_deref().is_some_and(|expected| expected != session) => {
            Some(TurnEnd::failed("session mismatch: worker returned an unexpected session id"))
        }
        _ if outcome.sessions.len() > 1 => Some(TurnEnd::failed("worker reported multiple session ids")),
        _ => None,
    }
}

fn opencode_end(
    task: &Task,
    code: Option<i32>,
    outcome: &TurnOutcome,
    route: Option<&RouteCheck>,
    stderr_tail: &str,
    now_ms: u64,
) -> TurnEnd {
    // Only a turn that went wrong can be a refusal: a limit named on stderr must
    // not block a worker that finished its work.
    let went_wrong = !outcome.errors.is_empty() || code != Some(0) || outcome.text.is_empty();
    if went_wrong && let Some(refusal) = quota::opencode_refusal(&outcome.raw_errors, stderr_tail, now_ms) {
        return TurnEnd { state: TaskState::Failed, error: Some(refusal.message.clone()), refusal: Some(refusal) };
    }
    if !outcome.errors.is_empty() {
        // Before the exit code: OpenCode v2 exits 0 after an error event.
        return TurnEnd::failed(format!("worker reported an error event: {}", outcome.errors[0]));
    }
    if code != Some(0) {
        let words = match code {
            Some(code) => format!("worker exited with code {code}"),
            None => "worker exited without an exit code".into(),
        };
        return TurnEnd::failed(words);
    }
    if outcome.parse_errors > 0 {
        return TurnEnd::failed(format!("worker stream contained {} unparseable lines", outcome.parse_errors));
    }
    if let Some(end) = session_end(task, outcome) {
        return end;
    }
    if outcome.text.is_empty() {
        return TurnEnd::failed("worker produced no final text");
    }
    match route {
        None => TurnEnd::failed("model verification error: the runner wrote no route check"),
        Some(check) if check.status == "verified" || route_unchecked(Some(check)).is_some() => {
            TurnEnd { state: TaskState::Done, error: None, refusal: None }
        }
        Some(check) => TurnEnd::failed(match &check.message {
            Some(message) if !message.is_empty() => format!("model verification {}: {message}", check.status),
            _ => format!("model verification {}", check.status),
        }),
    }
}

/// What a finished turn's route check could not say. An export that failed or came
/// back cut off is no evidence about the route, so the work stays done and the line
/// says it wasn't checked. A complete export naming another route still fails the task.
pub fn route_unchecked(route: Option<&RouteCheck>) -> Option<String> {
    let check = route?;
    if check.status != "error" && check.status != "unverifiable" {
        return None;
    }
    let words = check.message.as_deref().filter(|message| !message.is_empty()).unwrap_or(check.status.as_str());
    Some(format!("which model ran could not be checked: {words}"))
}

fn codex_end(task: &Task, code: Option<i32>, outcome: &TurnOutcome, now_ms: u64, offset_min: i32) -> TurnEnd {
    // Only a turn that went wrong can be a refusal: a limit named in a warning
    // must not block a worker that finished its work.
    let went_wrong = !outcome.errors.is_empty() || code != Some(0) || outcome.text.is_empty();
    if went_wrong && let Some(refusal) = quota::codex_refusal(&outcome.errors, now_ms, offset_min) {
        return TurnEnd { state: TaskState::Failed, error: Some(refusal.message.clone()), refusal: Some(refusal) };
    }
    if !outcome.errors.is_empty() {
        return TurnEnd::failed(format!("worker reported an error event: {}", outcome.errors[0]));
    }
    if code != Some(0) {
        let words = match code {
            Some(code) => format!("worker exited with code {code}"),
            None => "worker exited without an exit code".into(),
        };
        return TurnEnd::failed(words);
    }
    if outcome.parse_errors > 0 {
        return TurnEnd::failed(format!("worker stream contained {} unparseable lines", outcome.parse_errors));
    }
    if let Some(end) = session_end(task, outcome) {
        return end;
    }
    if outcome.text.is_empty() {
        return TurnEnd::failed("worker produced no final text");
    }
    TurnEnd { state: TaskState::Done, error: None, refusal: None }
}

fn command_code_end(
    task: &Task,
    route: &str,
    code: Option<i32>,
    outcome: &TurnOutcome,
    stderr_tail: &str,
    now_ms: u64,
    offset_min: i32,
) -> TurnEnd {
    if outcome.parse_errors > 0 {
        return TurnEnd::failed("worker stream contained unparseable lines");
    }
    let Some(reported) = &outcome.reported else {
        return TurnEnd::failed("worker stream ended without a result line");
    };
    let subtype = reported.subtype.to_lowercase();
    if subtype != "success" || code != Some(0) {
        // The CLI reports limits on stderr and in the result's error, never
        // only in the worker's own reply: reading the reply would let a failed
        // run that mentions "rate limit" block the worker.
        let blob = if reported.error.is_empty() {
            stderr_tail.to_string()
        } else {
            format!("{}\n{stderr_tail}", reported.error)
        };
        if let Some(refusal) = quota::command_code_refusal(&blob, true, now_ms, offset_min) {
            return TurnEnd { state: TaskState::Failed, error: Some(refusal.message.clone()), refusal: Some(refusal) };
        }
        if subtype == "max_turns" || code == Some(8) {
            return TurnEnd::failed("max_turns: Command Code stopped at --max-turns 200 before a final answer");
        }
        let detail = crate::task::types::cut(reported.error.trim(), 1000);
        let words = match code {
            Some(code) => format!("worker exited with code {code}"),
            None => "worker exited without an exit code".into(),
        };
        return TurnEnd::failed(if detail.is_empty() { words } else { format!("{words}: {detail}") });
    }
    if let Some(end) = session_end(task, outcome) {
        return end;
    }
    if outcome.text.is_empty() {
        return TurnEnd::failed("worker produced no final text");
    }
    let (model, _) = crate::task::commandcode::route(route);
    if outcome.models.is_empty() {
        return TurnEnd::failed("model verification unverifiable");
    }
    if let Some(seen) = outcome.models.iter().find(|seen| **seen != model) {
        return TurnEnd::failed(format!("model verification mismatch: {seen}"));
    }
    TurnEnd { state: TaskState::Done, error: None, refusal: None }
}

/// The state a turn leaves behind: a normal end with a question asks, any other
/// normal end checks, and a failed end with changes checks once so its work is
/// judged. Anything else keeps the end it was given.
pub fn after_turn(state: TaskState, text: &str, changed: bool) -> (TaskState, Option<String>) {
    if state == TaskState::Failed && changed {
        return (TaskState::Checking, None);
    }
    if state != TaskState::Done {
        return (state, None);
    }
    match question(text) {
        Some(question) => (TaskState::Asking, Some(question)),
        None => (TaskState::Checking, None),
    }
}

/// The words a wall-limit end leaves for its one report turn.
pub const REPORT_PROMPT: &str = "Your time is up. Change nothing more. Write your report now: what you changed, by job; \
                                 the commands you ran and their last lines; Tests updated; and Gaps, including what \
                                 you didn't finish.";

/// The first wall-limit end with changes leaves one more turn for the report.
pub fn report_turn_due(ending: Option<Ending>, changed: bool, queued: bool) -> bool {
    ending == Some(Ending::Wall) && changed && !queued
}

/// The body of the last `## Question` heading in a turn's final text, up to the
/// next level-two heading. An empty body, or no heading, is no question.
pub fn question(text: &str) -> Option<String> {
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.iter().rposition(|line| line.trim_end() == "## Question")? + 1;
    let end = (start..lines.len()).find(|at| lines[*at].starts_with("## ")).unwrap_or(lines.len());
    let body = lines[start..end].join("\n").trim().to_string();
    (!body.is_empty()).then_some(body)
}

/// Workers starting together collide (Command Code rewrites its config as it
/// starts, and OpenCode dies on a shared server): every start is at least 8 s
/// after the last one. The last start is kept in memory by the pump.
pub fn start_slot(last_start_ms: Option<u64>, now_ms: u64) -> Result<(), String> {
    match last_start_ms {
        Some(last) if now_ms.saturating_sub(last) < 8_000 => {
            Err("waiting for a start slot: workers start 8 s apart".into())
        }
        _ => Ok(()),
    }
}

/// The wait ends when every task has, when any ended badly (UPGRADES #51),
/// or, with `first`, at the first one that ended. A stranded task ends the
/// wait at once unless the caller asked to wait while every worker is out.
pub fn wait_over(lines: &[TaskLine], first: bool, while_stranded: bool) -> bool {
    if lines.is_empty() {
        return false;
    }
    if !while_stranded && lines.iter().any(|line| line.state == TaskState::Stranded) {
        return true;
    }
    if lines.iter().any(|line| line.state.ended_badly()) {
        return true;
    }
    let finished = |line: &TaskLine| line.state.ended() || line.state == TaskState::Asking;
    if first && lines.iter().any(finished) {
        return true;
    }
    lines.iter().all(finished)
}

#[cfg(test)]
mod tests;
