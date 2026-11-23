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
    assert_eq!(result(judge(&t, None, None, &good)), ended(TaskState::Failed, Some("the core stopped while it ran")));
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
    assert_eq!(end.error.as_deref(), Some("reached your weekly usage limit"), "a refusal comes before the error event");
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
        after_turn(TaskState::Done, "done\n\n## Question\nWhich?\n- A\n- B", false),
        (TaskState::Asking, Some("Which?\n- A\n- B".into()))
    );
    assert_eq!(after_turn(TaskState::Done, "done, no question", false), (TaskState::Checking, None));
    assert_eq!(after_turn(TaskState::Failed, "## Question\nWhich?", false), (TaskState::Failed, None));
}

#[test]
fn a_failed_turn_with_changes_is_checked_once() {
    assert_eq!(after_turn(TaskState::Failed, "boom", true), (TaskState::Checking, None));
    assert_eq!(
        after_turn(TaskState::Failed, "boom", false),
        (TaskState::Failed, None),
        "a failed turn with no changes runs nothing, as today"
    );
}

#[test]
fn a_wall_limit_queues_one_report_turn_only_with_changes() {
    assert!(report_turn_due(Some(Ending::Wall), true, false));
    assert!(!report_turn_due(Some(Ending::Wall), false, false), "a wall end with no changes queues nothing");
    assert!(!report_turn_due(Some(Ending::Wall), true, true), "a second wall end never queues another");
    assert!(!report_turn_due(Some(Ending::Idle), true, false), "an idle end keeps today's behaviour");
    assert!(!report_turn_due(None, true, false), "only the wall limit asks for a report");
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
