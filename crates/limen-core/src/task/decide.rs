//! What a task's record and the world mean for it: is a path in scope, may a
//! queued task start, is it time to cut a turn, and how did that turn end.
//! Pure: no IO, no processes, so every rule is a unit test.

use crate::task::types::{Beat, Ending, Task, TaskLine, TaskState};
use crate::task::worker::TurnOutcome;

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
pub fn may_start(task: &Task, others: &[&Task], cap: usize) -> Result<(), String> {
    let running = others.iter().filter(|other| other.state == TaskState::Running).count();
    if running >= cap {
        return Err(format!("waiting for a task to finish: {running} of {cap} running"));
    }
    for other in others {
        if other.name == task.name || other.repo != task.repo {
            continue;
        }
        let earlier = other.state == TaskState::Running || (other.state == TaskState::Queued && other.seq < task.seq);
        if earlier && scopes_overlap(&task.scope, &other.scope) {
            return Err(if other.state == TaskState::Running {
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
    if now_ms.saturating_sub(turn.started_ms) > task.wall_s.saturating_mul(1000) {
        return Step::Kill(Ending::Wall);
    }
    let active = beat.map_or(turn.started_ms, |beat| beat.active_ms.max(turn.started_ms));
    if now_ms.saturating_sub(active) > task.idle_s.saturating_mul(1000) {
        return Step::Kill(Ending::Idle);
    }
    Step::Wait
}

/// The end of a turn, in the order `execute_turn` uses in the Python: an
/// explicit end first, then what the terminal said, then the stream's facts.
pub fn turn_end(
    task: &Task,
    terminal: Option<&TerminalNow>,
    exit_file_code: Option<i32>,
    outcome: &TurnOutcome,
) -> (TaskState, Option<String>) {
    match task.ending {
        Some(Ending::Cancel) => return (TaskState::Cancelled, None),
        Some(Ending::Idle) => {
            let tool = outcome.tools.last().cloned().unwrap_or_else(|| "none".into());
            return (
                TaskState::TimedOut,
                Some(format!(
                    "no output and no CPU for {} s (idle limit {} s), last tool: {tool}",
                    task.idle_s, task.idle_s
                )),
            );
        }
        Some(Ending::Wall) => {
            return (TaskState::TimedOut, Some(format!("ran past its {} s limit", task.wall_s)));
        }
        None => {}
    }
    let code = terminal.and_then(|terminal| terminal.exit_code).or(exit_file_code);
    match terminal {
        None if code.is_none() => return (TaskState::Failed, Some("the core stopped while it ran".into())),
        Some(terminal) if terminal.reason.as_deref().is_some_and(|reason| reason != "exited") => {
            let words = match terminal.reason.as_deref().unwrap_or_default() {
                "killed" => "its terminal was killed",
                "server_stop" | "stopped_by_restart" => "the core stopped while it ran",
                _ => "its terminal was lost",
            };
            return (TaskState::Failed, Some(words.into()));
        }
        Some(terminal) if !terminal.live && code.is_none() => {
            return (TaskState::Failed, Some("its terminal was lost".into()));
        }
        _ => {}
    }
    if !outcome.errors.is_empty() {
        // Before the exit code: OpenCode v2 exits 0 after an error event.
        return (TaskState::Failed, Some(format!("worker reported an error event: {}", outcome.errors[0])));
    }
    if code != Some(0) {
        let words = match code {
            Some(code) => format!("worker exited with code {code}"),
            None => "worker exited without an exit code".into(),
        };
        return (TaskState::Failed, Some(words));
    }
    if outcome.parse_errors > 0 {
        return (
            TaskState::Failed,
            Some(format!("worker stream contained {} unparseable lines", outcome.parse_errors)),
        );
    }
    let expected = task.turns.iter().rev().skip(1).find_map(|turn| turn.session.clone());
    match &outcome.session {
        None => return (TaskState::Failed, Some("worker did not report a session id".into())),
        Some(session) if expected.as_deref().is_some_and(|expected| expected != session) => {
            return (TaskState::Failed, Some("session mismatch: worker returned an unexpected session id".into()));
        }
        _ if outcome.sessions.len() > 1 => {
            return (TaskState::Failed, Some("worker reported multiple session ids".into()));
        }
        _ => {}
    }
    if outcome.text.is_empty() {
        return (TaskState::Failed, Some("worker produced no final text".into()));
    }
    (TaskState::Done, None)
}

/// The wait ends when every task has, when any ended badly (UPGRADES #51),
/// or, with `first`, at the first one that ended.
pub fn wait_over(lines: &[TaskLine], first: bool) -> bool {
    if lines.is_empty() {
        return false;
    }
    if lines.iter().any(|line| line.state.ended_badly()) {
        return true;
    }
    if first && lines.iter().any(|line| line.state.ended()) {
        return true;
    }
    lines.iter().all(|line| line.state.ended())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::types::{ChangedFile, Turn, Usage};

    fn scopes(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
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
        assert_eq!(may_start(&third, &[&running, &run], 2), Err("waiting for a task to finish: 2 of 2 running".into()));
        let queued = task("d", TaskState::Queued, 4, &["src/a/**"]);
        let earlier = task("a", TaskState::Queued, 1, &["src/**"]);
        assert_eq!(may_start(&queued, &[&earlier, &run], 5), Err("queued behind a: its scope overlaps".into()));
        let disjoint = task("e", TaskState::Queued, 5, &["lib/**"]);
        assert!(may_start(&disjoint, &[&earlier, &run], 5).is_ok());
        let other_repo = Task { repo: "/elsewhere".into(), ..task("a", TaskState::Running, 1, &["src/**"]) };
        assert!(may_start(&third, &[&other_repo], 5).is_ok(), "another repo never overlaps");
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

    fn ended(state: TaskState, error: Option<&str>) -> (TaskState, Option<String>) {
        (state, error.map(String::from))
    }

    #[test]
    fn a_turn_ends_in_the_python_order() {
        let mut t = task("a", TaskState::Running, 1, &[]);
        t.turns = vec![turn(0)];
        t.idle_s = 2;
        t.wall_s = 10;
        let exited = TerminalNow { live: false, exit_code: Some(0), reason: Some("exited".into()) };
        let good = outcome();
        assert_eq!(turn_end(&t, Some(&exited), Some(0), &good), ended(TaskState::Done, None));

        t.ending = Some(Ending::Cancel);
        assert_eq!(turn_end(&t, Some(&exited), Some(0), &good), ended(TaskState::Cancelled, None));
        t.ending = Some(Ending::Idle);
        assert_eq!(
            turn_end(&t, Some(&exited), Some(0), &good),
            ended(TaskState::TimedOut, Some("no output and no CPU for 2 s (idle limit 2 s), last tool: none"))
        );
        t.ending = Some(Ending::Wall);
        assert_eq!(
            turn_end(&t, Some(&exited), Some(0), &good),
            ended(TaskState::TimedOut, Some("ran past its 10 s limit"))
        );
        t.ending = None;

        let killed = TerminalNow { live: false, exit_code: None, reason: Some("killed".into()) };
        assert_eq!(turn_end(&t, Some(&killed), None, &good), ended(TaskState::Failed, Some("its terminal was killed")));
        assert_eq!(turn_end(&t, None, None, &good), ended(TaskState::Failed, Some("the core stopped while it ran")));
        assert_eq!(
            turn_end(&t, None, Some(3), &good),
            ended(TaskState::Failed, Some("worker exited with code 3")),
            "a terminal gone with an exit file still reports the code"
        );

        let mut errors = good.clone();
        errors.errors = vec!["ProviderError (429): quota".into()];
        assert_eq!(
            turn_end(&t, Some(&exited), Some(0), &errors),
            ended(TaskState::Failed, Some("worker reported an error event: ProviderError (429): quota"))
        );
        let no_code = TerminalNow { live: false, exit_code: None, reason: Some("exited".into()) };
        assert_eq!(
            turn_end(&t, Some(&no_code), Some(3), &good),
            ended(TaskState::Failed, Some("worker exited with code 3")),
            "the exit file speaks when the terminal has no code"
        );
        let mut parse = good.clone();
        parse.parse_errors = 2;
        assert_eq!(
            turn_end(&t, Some(&exited), Some(0), &parse),
            ended(TaskState::Failed, Some("worker stream contained 2 unparseable lines"))
        );
        let mut no_session = good.clone();
        no_session.session = None;
        assert_eq!(
            turn_end(&t, Some(&exited), Some(0), &no_session),
            ended(TaskState::Failed, Some("worker did not report a session id"))
        );
        let mut two = good.clone();
        two.sessions.push("ses_2".into());
        assert_eq!(
            turn_end(&t, Some(&exited), Some(0), &two),
            ended(TaskState::Failed, Some("worker reported multiple session ids"))
        );
        let mut no_text = good.clone();
        no_text.text.clear();
        assert_eq!(
            turn_end(&t, Some(&exited), Some(0), &no_text),
            ended(TaskState::Failed, Some("worker produced no final text"))
        );

        // Continuing now expects the session the previous turn reported.
        t.turns[0].session = Some("ses_1".into());
        t.turns.push(turn(0));
        let mut other = good;
        other.session = Some("ses_2".into());
        other.sessions = vec!["ses_2".into()];
        assert_eq!(
            turn_end(&t, Some(&exited), Some(0), &other),
            ended(TaskState::Failed, Some("session mismatch: worker returned an unexpected session id"))
        );
    }

    fn line(name: &str, state: TaskState) -> TaskLine {
        TaskLine { task: name.into(), state, ..Default::default() }
    }

    #[test]
    fn a_wait_ends_when_all_end_a_bad_end_or_with_first_any_end() {
        let running = [line("a", TaskState::Running), line("b", TaskState::Running)];
        assert!(!wait_over(&running, false) && !wait_over(&running, true));
        assert!(!wait_over(&[], false), "nothing known is not everything ended");
        let first_done = [line("a", TaskState::Done), line("b", TaskState::Running)];
        assert!(!wait_over(&first_done, false));
        assert!(wait_over(&first_done, true));
        let bad = [line("a", TaskState::Failed), line("b", TaskState::Running)];
        assert!(wait_over(&bad, false), "a bad end wakes at once (UPGRADES #51)");
        let all = [line("a", TaskState::Done), line("b", TaskState::TimedOut)];
        assert!(wait_over(&all, false));
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
        assert!(TaskLine::of(&t, 1, 1500).ok);
    }
}
