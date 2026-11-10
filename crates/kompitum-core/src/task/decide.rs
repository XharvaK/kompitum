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
    Ok(())
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
/// normal end checks. Anything else keeps the end it was given.
pub fn after_turn(state: TaskState, text: &str) -> (TaskState, Option<String>) {
    if state != TaskState::Done {
        return (state, None);
    }
    match question(text) {
        Some(question) => (TaskState::Asking, Some(question)),
        None => (TaskState::Checking, None),
    }
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
mod tests {
    use super::*;
    use crate::task::types::{ChangedFile, Turn, Usage};

    fn scopes(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn workers() -> Vec<Worker> {
        crate::task::worker::builtin()
    }

    #[test]
    fn a_scope_admits_the_whole_folder_below_it_and_star_crosses_a_slash() {
        for path in ["src/a.py", "src/a/b.py", "src/a/b/c.py"] {
            assert!(in_scope(path, &scopes(&["src/**"])), "{path}");
            assert!(in_scope(path, &scopes(&["src"])), "{path}: a bare folder admits below it");
            assert!(in_scope(path, &scopes(&["src/*"])), "{path}: * crosses /");
        }
        assert!(in_scope("src/a.py", &scopes(&["src/*"])));
        assert!(!in_scope("other/src/a.py", &scopes(&["src/**"])));
        assert!(!in_scope("src/a.py", &scopes(&["src/a/**"])));
        assert!(in_scope("src/a/x.py", &scopes(&["src/a/**"])));
        assert!(in_scope("src\\a\\x.py", &scopes(&["src/a/**"])), "Windows separators read as /");
        assert!(in_scope("docs/x.md", &scopes(&["*.md"])), "a pattern matches the whole path");
        assert!(!in_scope("docs/x.txt", &scopes(&["*.md"])));
        assert!(in_scope("src/t7.rs", &scopes(&["src/t[0-9].rs"])));
        assert!(!in_scope("src/tx.rs", &scopes(&["src/t[0-9].rs"])));
        assert!(in_scope("anything", &[]), "an empty scope is the whole repo");
        assert!(!in_scope("anything", &scopes(&["/src/"])));
    }

    #[test]
    fn two_scopes_overlap_only_when_their_fixed_paths_agree() {
        assert!(!scopes_overlap(&scopes(&["src/a/**"]), &scopes(&["src/b/**"])));
        assert!(scopes_overlap(&scopes(&["src/a/**"]), &scopes(&["src/a/x.py"])));
        assert!(scopes_overlap(&scopes(&["src/a/**"]), &scopes(&["src/a"])));
        assert!(!scopes_overlap(&scopes(&["src/a"]), &scopes(&["src/ab"])));
        assert!(scopes_overlap(&[], &scopes(&["docs/**"])), "empty is the whole repo");
        assert!(scopes_overlap(&scopes(&["**"]), &scopes(&["anything"])));
        assert!(scopes_overlap(&scopes(&["src/*"]), &scopes(&["src/a/b"])));
        assert_eq!(static_prefix("src/a/**/x"), ["src", "a"]);
        assert!(static_prefix("**").is_empty());
    }

    fn task(name: &str, state: TaskState, seq: u64, scope: &[&str]) -> Task {
        Task { name: name.into(), seq, state, repo: "/repo".into(), scope: scopes(scope), ..Default::default() }
    }

    fn turn(started_ms: u64) -> Turn {
        Turn { started_ms, ..Default::default() }
    }

    #[test]
    fn the_cap_and_an_overlapping_earlier_task_hold_a_queue_in_order() {
        let third = task("c", TaskState::Queued, 3, &["src/a/**"]);
        let running = task("a", TaskState::Running, 1, &["src/a/**"]);
        let run = task("b", TaskState::Running, 2, &["docs/**"]);
        assert_eq!(
            may_start(&third, &[&running, &run], Some(2)),
            Err("waiting for a task to finish: 2 of 2 running".into())
        );
        assert!(
            may_start(&task("f", TaskState::Queued, 6, &["lib/**"]), &[&running, &run], None).is_ok(),
            "no cap holds nothing back"
        );
        let queued = task("d", TaskState::Queued, 4, &["src/a/**"]);
        let earlier = task("a", TaskState::Queued, 1, &["src/**"]);
        assert_eq!(may_start(&queued, &[&earlier, &run], Some(5)), Err("queued behind a: its scope overlaps".into()));
        let disjoint = task("e", TaskState::Queued, 5, &["lib/**"]);
        assert!(may_start(&disjoint, &[&earlier, &run], Some(5)).is_ok());
        let other_repo = Task { repo: "/elsewhere".into(), ..task("a", TaskState::Running, 1, &["src/**"]) };
        assert!(may_start(&third, &[&other_repo], Some(5)).is_ok(), "another repo never overlaps");
        let stranded = task("s", TaskState::Stranded, 1, &["src/**"]);
        assert_eq!(may_start(&queued, &[&stranded], Some(5)), Err("queued behind s: its scope overlaps".into()));
        assert_eq!(may_start(&disjoint, &[&stranded], Some(5)), Ok(()), "stranded is not running");
    }

    #[test]
    fn one_at_a_time_holds_disjoint_scopes_in_one_repository() {
        let running = task("a", TaskState::Running, 1, &["src/**"]);
        let mut next = task("b", TaskState::Queued, 2, &["docs/**"]);
        next.one_at_a_time = true;
        assert_eq!(
            may_start(&next, &[&running], Some(5)),
            Err("waiting for a: this repository builds one task at a time".into())
        );
        next.one_at_a_time = false;
        assert!(may_start(&next, &[&running], Some(5)).is_ok(), "without the flag, disjoint scopes start");

        let earlier = task("a", TaskState::Queued, 1, &["src/**"]);
        let mut next = task("b", TaskState::Queued, 2, &["docs/**"]);
        next.one_at_a_time = true;
        assert_eq!(
            may_start(&next, &[&earlier], Some(5)),
            Err("queued behind a: this repository builds one task at a time".into())
        );
        let other_repo = Task { repo: "/elsewhere".into(), ..running };
        assert!(may_start(&next, &[&other_repo], Some(5)).is_ok(), "another repository is not held");

        let asking = task("a", TaskState::Asking, 1, &["src/**"]);
        let overlapping = task("b", TaskState::Queued, 2, &["src/a/**"]);
        assert_eq!(
            may_start(&overlapping, &[&asking], Some(1)),
            Err("waiting for a: it is asking, and its scope overlaps".into()),
            "an asking task keeps its scope, and takes no slot of the cap"
        );
        let mut builder = task("c", TaskState::Queued, 3, &["docs/**"]);
        builder.one_at_a_time = true;
        assert!(may_start(&builder, &[&asking], Some(1)).is_ok(), "an asking task holds no one-at-a-time slot");
    }

    #[test]
    fn a_turn_ends_with_its_terminal_or_at_its_limits() {
        let mut t = task("a", TaskState::Running, 1, &[]);
        t.turns = vec![turn(1_000)];
        t.idle_s = 2;
        t.wall_s = 10;
        let live = TerminalNow { live: true, ..Default::default() };
        let beat = Beat { active_ms: 4_000, ..Default::default() };
        assert_eq!(step(&t, Some(&live), Some(&beat), 5_000), Step::Wait);
        assert_eq!(step(&t, Some(&live), Some(&beat), 6_001), Step::Kill(Ending::Idle));
        assert_eq!(step(&t, Some(&live), None, 1_500), Step::Wait, "no beat: the turn's start");
        assert_eq!(step(&t, Some(&live), None, 3_001), Step::Kill(Ending::Idle));
        assert_eq!(step(&t, Some(&live), Some(&beat), 11_001), Step::Kill(Ending::Wall));
        t.wall_s = 0;
        let ten_hours = 10 * 3_600_000u64;
        let still_active = Beat { active_ms: ten_hours, ..Default::default() };
        assert_eq!(
            step(&t, Some(&live), Some(&still_active), ten_hours + 1),
            Step::Wait,
            "a turn ten hours old is not cut when no wall limit is set and it is still active"
        );
        t.wall_s = 10;
        assert_eq!(step(&t, None, Some(&beat), 5_000), Step::TurnOver);
        assert_eq!(step(&t, Some(&TerminalNow { live: false, ..Default::default() }), None, 5_000), Step::TurnOver);
        t.ending = Some(Ending::Cancel);
        assert_eq!(step(&t, Some(&live), Some(&beat), 5_000), Step::Kill(Ending::Cancel), "a lost kill goes again");
        assert_eq!(step(&t, None, Some(&beat), 5_000), Step::TurnOver);
    }

    fn outcome() -> TurnOutcome {
        TurnOutcome {
            session: Some("ses_1".into()),
            sessions: vec!["ses_1".into()],
            text: "all done".into(),
            ..Default::default()
        }
    }

    fn verified() -> RouteCheck {
        RouteCheck {
            status: "verified".into(),
            expected: "opencode-go/deepseek-v4.1-flash#max".into(),
            ..Default::default()
        }
    }

    fn judge(t: &Task, terminal: Option<&TerminalNow>, code: Option<i32>, outcome: &TurnOutcome) -> TurnEnd {
        turn_end(t, &workers(), terminal, code, outcome, Some(&verified()), "", 1_000, 0)
    }

    fn ended(state: TaskState, error: Option<&str>) -> (TaskState, Option<String>) {
        (state, error.map(String::from))
    }

    fn result(end: TurnEnd) -> (TaskState, Option<String>) {
        (end.state, end.error)
    }

    #[test]
    fn a_turn_ends_in_the_python_order() {
        let mut t = task("a", TaskState::Running, 1, &[]);
        t.turns = vec![turn(0)];
        t.idle_s = 2;
        t.wall_s = 10;
        let exited = TerminalNow { live: false, exit_code: Some(0), reason: Some("exited".into()) };
        let good = outcome();
        assert_eq!(result(judge(&t, Some(&exited), Some(0), &good)), ended(TaskState::Done, None));

        t.ending = Some(Ending::Cancel);
        assert_eq!(result(judge(&t, Some(&exited), Some(0), &good)), ended(TaskState::Cancelled, None));
        t.ending = Some(Ending::Idle);
        assert_eq!(
            result(judge(&t, Some(&exited), Some(0), &good)),
            ended(TaskState::TimedOut, Some("no output, CPU or I/O for 2 s (idle limit 2 s), last tool: none"))
        );
        t.ending = Some(Ending::Wall);
        assert_eq!(
            result(judge(&t, Some(&exited), Some(0), &good)),
            ended(TaskState::TimedOut, Some("ran past its 10 s limit"))
        );
        t.ending = None;

        let killed = TerminalNow { live: false, exit_code: None, reason: Some("killed".into()) };
        assert_eq!(
            result(judge(&t, Some(&killed), None, &good)),
            ended(TaskState::Failed, Some("its terminal was killed"))
        );
        assert_eq!(
            result(judge(&t, None, None, &good)),
            ended(TaskState::Failed, Some("the core stopped while it ran"))
        );
        assert_eq!(
            result(judge(&t, None, Some(3), &good)),
            ended(TaskState::Failed, Some("worker exited with code 3")),
            "a terminal gone with an exit file still reports the code"
        );

        // OpenCode, in the order `execute_turn` uses.
        let mut quota = good.clone();
        quota.errors = vec!["ProviderError (429): quota".into()];
        quota.raw_errors = vec![serde_json::json!({
            "name": "ProviderError", "status": 429, "message": "reached your weekly usage limit"})];
        let end = turn_end(&t, &workers(), Some(&exited), Some(3), &quota, Some(&verified()), "", 1_000, 0);
        assert_eq!(end.state, TaskState::Failed);
        assert_eq!(end.refusal.as_ref().unwrap().window, "weekly");
        assert_eq!(
            end.error.as_deref(),
            Some("reached your weekly usage limit"),
            "a refusal comes before the error event"
        );
        let mut errors = good.clone();
        errors.errors = vec!["ProviderError (429): quota".into()];
        assert_eq!(
            result(judge(&t, Some(&exited), Some(0), &errors)),
            ended(TaskState::Failed, Some("worker reported an error event: ProviderError (429): quota"))
        );
        let no_code = TerminalNow { live: false, exit_code: None, reason: Some("exited".into()) };
        assert_eq!(
            result(judge(&t, Some(&no_code), Some(3), &good)),
            ended(TaskState::Failed, Some("worker exited with code 3")),
            "the exit file speaks when the terminal has no code"
        );
        let mut parse = good.clone();
        parse.parse_errors = 2;
        assert_eq!(
            result(judge(&t, Some(&exited), Some(0), &parse)),
            ended(TaskState::Failed, Some("worker stream contained 2 unparseable lines"))
        );
        let mut no_session = good.clone();
        no_session.session = None;
        assert_eq!(
            result(judge(&t, Some(&exited), Some(0), &no_session)),
            ended(TaskState::Failed, Some("worker did not report a session id"))
        );
        let mut two = good.clone();
        two.sessions.push("ses_2".into());
        assert_eq!(
            result(judge(&t, Some(&exited), Some(0), &two)),
            ended(TaskState::Failed, Some("worker reported multiple session ids"))
        );
        let mut no_text = good.clone();
        no_text.text.clear();
        assert_eq!(
            result(judge(&t, Some(&exited), Some(0), &no_text)),
            ended(TaskState::Failed, Some("worker produced no final text"))
        );
        let no_route = turn_end(&t, &workers(), Some(&exited), Some(0), &good, None, "", 1_000, 0);
        assert_eq!(
            result(no_route),
            ended(TaskState::Failed, Some("model verification error: the runner wrote no route check"))
        );
        let mismatch = RouteCheck {
            status: "mismatch".into(),
            expected: "opencode-go/deepseek-v4.1-flash#max".into(),
            message: Some("asked for opencode-go/deepseek-v4.1-flash#max, the export says opencode/other".into()),
            ..Default::default()
        };
        let end = turn_end(&t, &workers(), Some(&exited), Some(0), &good, Some(&mismatch), "", 1_000, 0);
        assert_eq!(end.state, TaskState::Failed);
        assert!(end.error.unwrap().starts_with("model verification mismatch: asked for"));
        assert_eq!(route_unchecked(Some(&mismatch)), None);

        // A check that could not run says nothing about the route, and the work is there.
        for status in ["error", "unverifiable"] {
            let unknown =
                RouteCheck { status: status.into(), message: Some("export timed out".into()), ..Default::default() };
            let end = turn_end(&t, &workers(), Some(&exited), Some(0), &good, Some(&unknown), "", 1_000, 0);
            assert_eq!(end.state, TaskState::Done, "{status}");
            assert_eq!(end.error, None, "{status}");
            assert_eq!(
                route_unchecked(Some(&unknown)).as_deref(),
                Some("which model ran could not be checked: export timed out")
            );
        }
        assert_eq!(route_unchecked(None), None);

        // A turn that resumes a session expects exactly that one.
        t.turns.push(Turn { resumes: Some("ses_1".to_string()), ..turn(0) });
        let mut other = good;
        other.session = Some("ses_2".into());
        other.sessions = vec!["ses_2".into()];
        assert_eq!(
            result(judge(&t, Some(&exited), Some(0), &other)),
            ended(TaskState::Failed, Some("session mismatch: worker returned an unexpected session id"))
        );
    }

    #[test]
    fn a_session_mismatch_counts_only_for_a_turn_that_resumed() {
        let exited = TerminalNow { live: false, exit_code: Some(0), reason: Some("exited".into()) };
        let mut t = task("a", TaskState::Running, 1, &[]);
        t.turns = vec![turn(0)];
        let fresh = outcome();
        assert_eq!(result(judge(&t, Some(&exited), Some(0), &fresh)), ended(TaskState::Done, None));

        t.turns.push(Turn { resumes: Some("ses_1".to_string()), ..turn(0) });
        assert_eq!(result(judge(&t, Some(&exited), Some(0), &fresh)), ended(TaskState::Done, None));
        let mut other = fresh;
        other.session = Some("ses_2".into());
        other.sessions = vec!["ses_2".into()];
        assert_eq!(
            result(judge(&t, Some(&exited), Some(0), &other)),
            ended(TaskState::Failed, Some("session mismatch: worker returned an unexpected session id"))
        );
    }

    #[test]
    fn a_limit_named_on_stderr_counts_only_when_the_turn_went_wrong() {
        let mut t = task("a", TaskState::Running, 1, &[]);
        t.turns = vec![turn(0)];
        let exited = TerminalNow { live: false, exit_code: Some(0), reason: Some("exited".into()) };
        let stale = r#"{"type":"error","error":{"type":"FreeUsageLimitError"}}"#;
        let clean = turn_end(&t, &workers(), Some(&exited), Some(0), &outcome(), Some(&verified()), stale, 1_000, 0);
        assert_eq!((clean.state, clean.refusal), (TaskState::Done, None));
        let no_text = TurnOutcome { text: String::new(), ..outcome() };
        let refused = turn_end(&t, &workers(), Some(&exited), Some(0), &no_text, Some(&verified()), stale, 1_000, 0);
        assert_eq!(refused.state, TaskState::Failed);
        assert!(refused.refusal.is_some());
    }

    fn cc_task() -> Task {
        let mut t = task("a", TaskState::Running, 1, &[]);
        t.turns = vec![Turn { worker: "cc".into(), ..turn(0) }];
        t
    }

    fn cc_outcome() -> TurnOutcome {
        TurnOutcome {
            session: Some("s1".into()),
            sessions: vec!["s1".into()],
            text: "DONE".into(),
            reported: Some(crate::task::worker::Reported {
                subtype: "success".into(),
                error: String::new(),
                final_text: "DONE".into(),
            }),
            models: vec!["deepseek/deepseek-v4.1-flash".into()],
            ..Default::default()
        }
    }

    fn codex_task(resumes: Option<&str>) -> Task {
        let mut t = task("a", TaskState::Running, 1, &[]);
        t.turns =
            vec![Turn { worker: "codex".into(), cli: Some(Cli::Codex), resumes: resumes.map(String::from), ..turn(0) }];
        t
    }

    fn codex_outcome() -> TurnOutcome {
        TurnOutcome {
            session: Some("thread-1".into()),
            sessions: vec!["thread-1".into()],
            text: "DONE".into(),
            ..Default::default()
        }
    }

    #[test]
    fn a_codex_turn_ends_well_or_names_what_went_wrong() {
        let exited = TerminalNow { live: false, exit_code: Some(0), reason: Some("exited".into()) };
        let good = codex_outcome();
        assert_eq!(result(judge(&codex_task(None), Some(&exited), Some(0), &good)), ended(TaskState::Done, None));

        let mut failed = good.clone();
        failed.errors = vec!["the tool exploded".into()];
        failed.text.clear();
        assert_eq!(
            result(judge(&codex_task(None), Some(&exited), Some(1), &failed)),
            ended(TaskState::Failed, Some("worker reported an error event: the tool exploded"))
        );

        let mut refused = good;
        refused.errors = vec!["You’ve hit your usage limit. Try again later.".into()];
        refused.text.clear();
        let end = judge(&codex_task(None), Some(&exited), Some(0), &refused);
        assert_eq!(end.state, TaskState::Failed);
        assert_eq!(end.refusal.as_ref().unwrap().window, "5-hour");
    }

    #[test]
    fn a_resumed_codex_turn_must_report_the_same_thread() {
        let exited = TerminalNow { live: false, exit_code: Some(0), reason: Some("exited".into()) };
        let same = codex_outcome();
        assert_eq!(
            result(judge(&codex_task(Some("thread-1")), Some(&exited), Some(0), &same)),
            ended(TaskState::Done, None)
        );
        let mut other = codex_outcome();
        other.session = Some("thread-2".into());
        other.sessions = vec!["thread-2".into()];
        assert_eq!(
            result(judge(&codex_task(Some("thread-1")), Some(&exited), Some(0), &other)),
            ended(TaskState::Failed, Some("session mismatch: worker returned an unexpected session id"))
        );
    }

    #[test]
    fn a_command_code_turn_ends_in_the_python_order() {
        let t = cc_task();
        let exited = TerminalNow { live: false, exit_code: Some(0), reason: Some("exited".into()) };
        let good = cc_outcome();
        assert_eq!(result(judge(&t, Some(&exited), Some(0), &good)), ended(TaskState::Done, None));

        let mut parse = good.clone();
        parse.parse_errors = 1;
        assert_eq!(
            result(judge(&t, Some(&exited), Some(0), &parse)),
            ended(TaskState::Failed, Some("worker stream contained unparseable lines"))
        );
        let mut no_result = good.clone();
        no_result.reported = None;
        assert_eq!(
            result(judge(&t, Some(&exited), Some(0), &no_result)),
            ended(TaskState::Failed, Some("worker stream ended without a result line"))
        );
        let mut refused = good.clone();
        refused.reported = Some(crate::task::worker::Reported {
            subtype: "error".into(),
            error: "You have insufficient credits to make this request.".into(),
            final_text: String::new(),
        });
        let end = turn_end(&t, &workers(), Some(&exited), Some(1), &refused, None, "", 1_000, 0);
        assert_eq!(end.state, TaskState::Failed);
        assert_eq!(end.refusal.as_ref().unwrap().window, "credits");
        let mut max_turns = good.clone();
        max_turns.reported = Some(crate::task::worker::Reported {
            subtype: "max_turns".into(),
            error: String::new(),
            final_text: "partial".into(),
        });
        assert_eq!(
            result(judge(&t, Some(&exited), Some(0), &max_turns)),
            ended(TaskState::Failed, Some("max_turns: Command Code stopped at --max-turns 200 before a final answer"))
        );
        let mut failed = good.clone();
        failed.reported = Some(crate::task::worker::Reported {
            subtype: "error".into(),
            error: "the tool exploded".into(),
            final_text: String::new(),
        });
        let exited3 = TerminalNow { live: false, exit_code: Some(3), reason: Some("exited".into()) };
        assert_eq!(
            result(judge(&t, Some(&exited3), Some(3), &failed)),
            ended(TaskState::Failed, Some("worker exited with code 3: the tool exploded"))
        );
        let mut no_session = good.clone();
        no_session.session = None;
        assert_eq!(
            result(judge(&t, Some(&exited), Some(0), &no_session)),
            ended(TaskState::Failed, Some("worker did not report a session id"))
        );
        let mut two = good.clone();
        two.sessions.push("s2".into());
        assert_eq!(
            result(judge(&t, Some(&exited), Some(0), &two)),
            ended(TaskState::Failed, Some("worker reported multiple session ids"))
        );
        let mut no_text = good.clone();
        no_text.text.clear();
        assert_eq!(
            result(judge(&t, Some(&exited), Some(0), &no_text)),
            ended(TaskState::Failed, Some("worker produced no final text"))
        );
        let mut no_model = good.clone();
        no_model.models.clear();
        assert_eq!(
            result(judge(&t, Some(&exited), Some(0), &no_model)),
            ended(TaskState::Failed, Some("model verification unverifiable"))
        );
        let mut other_model = good;
        other_model.models = vec!["deepseek/other".into()];
        assert_eq!(
            result(judge(&t, Some(&exited), Some(0), &other_model)),
            ended(TaskState::Failed, Some("model verification mismatch: deepseek/other"))
        );
    }

    #[test]
    fn a_turn_whose_worker_was_removed_still_ends_with_its_own_cli() {
        let mut t = task("a", TaskState::Running, 1, &[]);
        t.turns = vec![Turn { worker: "gone".into(), cli: Some(Cli::CommandCode), ..turn(0) }];
        let exited = TerminalNow { live: false, exit_code: Some(0), reason: Some("exited".into()) };
        let end = turn_end(&t, &[], Some(&exited), Some(0), &TurnOutcome::default(), None, "", 1_000, 0);
        assert_eq!(
            end.error.as_deref(),
            Some("worker stream ended without a result line"),
            "the recorded CLI chose the Command Code reader"
        );
    }

    #[test]
    fn a_command_code_turn_checks_the_route_of_its_own_worker() {
        let mut t = task("a", TaskState::Running, 1, &[]);
        t.turns = vec![Turn { worker: "mine-cc".into(), ..turn(0) }];
        let mut workers = workers();
        workers.push(Worker {
            key: "mine-cc".into(),
            label: "Mine".into(),
            cli: Cli::CommandCode,
            route: "commandcode/vendor/model#high".into(),
        });
        let mut good = cc_outcome();
        good.models = vec!["vendor/model".into()];
        let exited = TerminalNow { live: false, exit_code: Some(0), reason: Some("exited".into()) };
        let end = turn_end(&t, &workers, Some(&exited), Some(0), &good, None, "", 1_000, 0);
        assert_eq!(result(end), ended(TaskState::Done, None), "the task's own route is the one checked");
    }

    #[test]
    fn a_question_is_the_body_under_the_last_exact_heading() {
        assert_eq!(
            question("work\n\n## Question\nWhich option?\n- A\n- B\n\n## Notes\nthe rest"),
            Some("Which option?\n- A\n- B".into())
        );
        assert_eq!(question("work but no question"), None);
        assert_eq!(question("## Question\n\n## Next\nwork"), None, "an empty body is no question");
        assert_eq!(
            question("## Question\nfirst\n## Question \nsecond"),
            Some("second".into()),
            "the last heading wins, and trailing spaces are allowed"
        );
        assert_eq!(question("## Questions\nis this one?"), None, "a longer word is not the heading");
        assert_eq!(question("## Question\nwork\n### A subheading stays"), Some("work\n### A subheading stays".into()));
    }

    #[test]
    fn a_normal_end_with_a_question_asks_and_checks_nothing() {
        assert_eq!(
            after_turn(TaskState::Done, "done\n\n## Question\nWhich?\n- A\n- B"),
            (TaskState::Asking, Some("Which?\n- A\n- B".into()))
        );
        assert_eq!(after_turn(TaskState::Done, "done, no question"), (TaskState::Checking, None));
        assert_eq!(after_turn(TaskState::Failed, "## Question\nWhich?"), (TaskState::Failed, None));
    }

    #[test]
    fn every_workers_start_is_eight_seconds_after_the_last() {
        assert!(start_slot(None, 1_000).is_ok());
        assert!(start_slot(Some(1_000), 1_001).is_err());
        assert_eq!(start_slot(Some(1_000), 8_999), Err("waiting for a start slot: workers start 8 s apart".into()));
        assert!(start_slot(Some(1_000), 9_000).is_ok());
        assert!(start_slot(Some(1_000), 30_000).is_ok());
    }

    fn line(name: &str, state: TaskState) -> TaskLine {
        TaskLine { task: name.into(), state, ..Default::default() }
    }

    #[test]
    fn a_wait_ends_when_all_end_a_bad_end_or_with_first_any_end() {
        let running = [line("a", TaskState::Running), line("b", TaskState::Running)];
        assert!(!wait_over(&running, false, false) && !wait_over(&running, true, false));
        assert!(!wait_over(&[], false, false), "nothing known is not everything ended");
        let first_done = [line("a", TaskState::Done), line("b", TaskState::Running)];
        assert!(!wait_over(&first_done, false, false));
        assert!(wait_over(&first_done, true, false));
        let bad = [line("a", TaskState::Failed), line("b", TaskState::Running)];
        assert!(wait_over(&bad, false, false), "a bad end wakes at once (UPGRADES #51)");
        let all = [line("a", TaskState::Done), line("b", TaskState::TimedOut)];
        assert!(wait_over(&all, false, false));
        let stranded = [line("a", TaskState::Stranded), line("b", TaskState::Running)];
        assert!(wait_over(&stranded, false, false), "stranded ends the wait unless asked to stay");
        assert!(wait_over(&stranded, true, false));
        assert!(!wait_over(&stranded, false, true), "while stranded, the task goes on by itself");
        assert!(!wait_over(&stranded, true, true));
        let asking = [line("a", TaskState::Asking), line("b", TaskState::Running)];
        assert!(wait_over(&asking, true, false), "first wakes at the first task that asks");
        assert!(!wait_over(&asking, false, false));
        let all_asking = [line("a", TaskState::Asking), line("b", TaskState::Done)];
        assert!(wait_over(&all_asking, false, false), "an asking task is as good as ended");
    }

    #[test]
    fn a_changed_file_marks_scope_and_ok_follows_it() {
        let file = ChangedFile {
            path: "src/a.rs".into(),
            added: 1,
            removed: 0,
            in_scope: in_scope("src/a.rs", &scopes(&["src/**"])),
        };
        assert!(file.in_scope);
        let mut t = task("a", TaskState::Done, 1, &["src/**"]);
        t.turns = vec![Turn { text: "ok".into(), usage: Usage::default(), ..turn(0) }];
        t.changed = vec![file];
        assert!(TaskLine::of(&t, &workers(), 1, 1500).ok);
    }
}
