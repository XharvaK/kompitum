//! A failed turn's checks, and the report turn at the time limit.

#[allow(unused_imports)]
use super::*;

const REPORT_PROMPT: &str = "Your time is up. Change nothing more. Write your report now: what you changed, by job; \
                              the commands you ran and their last lines; Tests updated; and Gaps, including what you \
                              didn't finish.";

fn error_event(session: &str) -> String {
    json!({"type": "error", "sessionID": session,
        "error": {"name": "ProviderError", "data": {"code": "x", "message": "boom"}}})
    .to_string()
}

/// An OpenCode turn that fails on an error event and still exits 0.
fn failed_after(actions: &mut Vec<Value>, session: &str) {
    actions.push(json!({"out": error_event(session)}));
    actions.push(json!({"out": stream(session, "text")[3]}));
    actions.push(json!({"exit": 0}));
}

#[test]
fn a_failed_turn_with_changes_records_its_checks_and_stays_failed() {
    let s = Session::new();
    let repo = s.repo();
    let mut actions = vec![json!({"write": ["made.txt", "hello"]})];
    failed_after(&mut actions, "ses_failed_checks");
    let script = s.script("failed-checks", json!([actions]));
    s.config(&script, "");
    s.start("fail-1", "write made.txt", &repo, &["--worker", "go"]);
    let (exit, rows, _) = s.wait(&["fail-1"], &["--timeout", "60"]);
    assert_eq!(exit, 1, "{rows:?}");
    assert_eq!(rows[0]["state"], "failed", "{rows:?}");
    assert!(rows[0]["error"].as_str().unwrap().contains("ProviderError"), "{rows:?}");

    let view = s.task("fail-1");
    let task = &view["task"];
    assert_eq!(task["turns"].as_array().unwrap().len(), 1, "no follow-up was queued: {view}");
    let checks = task["checks"].as_array().unwrap();
    assert_eq!(checks.len(), 1, "the changed work got one checks report: {view}");
    assert_eq!(checks[0]["turn"], 1, "{view}");
    assert_eq!(task["qa_used"], 0, "no QA round follows a failed turn: {view}");
    let folder = PathBuf::from(view["folder"].as_str().unwrap());
    assert!(!folder.join("turn-2.followup.md").exists(), "no follow-up file was written");
}

#[test]
fn a_failed_turn_with_no_changes_runs_no_checks() {
    let s = Session::new();
    let repo = s.repo();
    let mut actions = Vec::new();
    failed_after(&mut actions, "ses_failed_none");
    let script = s.script("failed-none", json!([actions]));
    s.config(&script, "");
    s.start("fail-2", "change nothing", &repo, &["--worker", "go"]);
    let (exit, rows, _) = s.wait(&["fail-2"], &["--timeout", "60"]);
    assert_eq!(exit, 1, "{rows:?}");
    assert_eq!(rows[0]["state"], "failed", "{rows:?}");

    let view = s.task("fail-2");
    assert!(view["task"]["checks"].as_array().unwrap().is_empty(), "no checks report: {view}");
    assert_eq!(view["task"]["turns"].as_array().unwrap().len(), 1, "{view}");
}

#[test]
fn a_wall_limit_with_changes_queues_one_report_turn_and_then_times_out() {
    let s = Session::new();
    let repo = s.repo();
    let first = vec![json!({"write": ["made.txt", "hello"]}), json!({"sleep_ms": 60000})];
    let second = say(&stream("ses_report", "The report: changed made.txt."));
    let script = s.script("wall-report", json!([first, second]));
    s.config(&script, "");
    s.start("wall-1", "work past the limit", &repo, &["--worker", "go", "--timeout", "2"]);
    let (exit, rows, _) = s.wait(&["wall-1"], &["--timeout", "60"]);
    assert_eq!(exit, 1, "{rows:?}");
    assert_eq!(rows[0]["state"], "timed_out", "{rows:?}");

    let view = s.task("wall-1");
    let task = &view["task"];
    let turns = task["turns"].as_array().unwrap();
    assert_eq!(turns.len(), 2, "one report turn followed the wall-limit end: {view}");
    assert_eq!(turns[1]["role"], "report", "{view}");
    assert_eq!(turns[1]["text"], "The report: changed made.txt.", "{view}");
    assert_eq!(task["wall_s"], 600, "the report turn got ten minutes: {view}");
    assert!(task["checks"].as_array().unwrap().is_empty(), "a timed-out task runs no checks: {view}");
    assert!(task["error"].as_str().unwrap().contains("2 s limit"), "it still says why: {view}");

    let folder = PathBuf::from(view["folder"].as_str().unwrap());
    assert_eq!(std::fs::read_to_string(folder.join("turn-2.followup.md")).unwrap(), REPORT_PROMPT);
}

#[test]
fn a_wall_limit_with_no_changes_queues_no_report_turn() {
    let s = Session::new();
    let repo = s.repo();
    let script = s.script("wall-none", json!([[{"sleep_ms": 60000}]]));
    s.config(&script, "");
    s.start("wall-2", "nothing to report", &repo, &["--worker", "go", "--timeout", "2"]);
    let (exit, rows, _) = s.wait(&["wall-2"], &["--timeout", "60"]);
    assert_eq!(exit, 1, "{rows:?}");
    assert_eq!(rows[0]["state"], "timed_out", "{rows:?}");

    let view = s.task("wall-2");
    assert_eq!(view["task"]["turns"].as_array().unwrap().len(), 1, "{view}");
    let folder = PathBuf::from(view["folder"].as_str().unwrap());
    assert!(!folder.join("turn-2.followup.md").exists(), "no report turn was queued");
}
