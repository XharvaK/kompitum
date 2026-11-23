//! A discarded task's name can be used again.

use super::*;

const REPORT: &str = "the worker's report\n\n## Gaps\nNone.\n";

fn turn(session: &str) -> Vec<Value> {
    let mut actions = say(&stream(session, REPORT));
    actions.push(json!({"exit": 0}));
    actions
}

/// Start one task, let it finish, and discard it.
fn run_and_discard(s: &Session, repo: &Path, name: &str) {
    let started = s.start(name, "do the work", repo, &["--worker", "go"]);
    assert!(matches!(started["state"].as_str(), Some("running" | "queued")), "{started}");
    let (exit, rows, summary) = s.wait(&[name], &["--timeout", "60"]);
    assert_eq!(exit, 0, "{rows:?} {summary}");
    let discarded = s.json(&["task", "discard", name]);
    assert_eq!(discarded["state"], "discarded", "{discarded}");
}

fn listed(s: &Session) -> Vec<String> {
    let list = s.json(&["task", "ls", "--all"]);
    list["tasks"].as_array().unwrap().iter().filter_map(|task| task["task"].as_str().map(String::from)).collect()
}

#[test]
fn a_discarded_tasks_name_starts_again_and_its_record_moves_to_the_first_suffix() {
    let s = Session::new();
    let repo = s.repo();
    let script = s.script("names", json!([turn("ses_names_1"), turn("ses_names_2")]));
    s.config(&script, "");
    run_and_discard(&s, &repo, "again");
    run_and_discard(&s, &repo, "again");

    let names = listed(&s);
    assert!(names.contains(&"again".to_string()), "the new task holds the name: {names:?}");
    assert!(names.contains(&"again.1".to_string()), "the old record moved to the first free suffix: {names:?}");
    assert!(s.state().join("tasks").join("again.1").join("record.json").is_file(), "the old folder moved whole");
    s.json(&["task", "discard", "again"]);
}

#[test]
fn a_second_discard_moves_the_older_record_to_the_next_free_suffix() {
    let s = Session::new();
    let repo = s.repo();
    let script = s.script("names-two", json!([turn("ses_names_a"), turn("ses_names_b"), turn("ses_names_c")]));
    s.config(&script, "");
    run_and_discard(&s, &repo, "twice");
    run_and_discard(&s, &repo, "twice");
    run_and_discard(&s, &repo, "twice");

    let names = listed(&s);
    assert!(names.contains(&"twice".to_string()), "the new task holds the name: {names:?}");
    assert!(names.contains(&"twice.1".to_string()), "the first record keeps its suffix: {names:?}");
    assert!(names.contains(&"twice.2".to_string()), "the second record takes the next free suffix: {names:?}");
    s.json(&["task", "discard", "twice"]);
}

#[test]
fn an_accepted_tasks_name_stays_taken_with_the_new_hint() {
    let s = Session::new();
    let repo = s.repo();
    let mut actions = vec![json!({"write": ["made.txt", "hello"]})];
    actions.extend(say(&stream("ses_names_accepted", REPORT)));
    actions.push(json!({"exit": 0}));
    let script = s.script("names-accepted", json!([actions]));
    s.config(&script, "");
    s.start("kept", "do the work", &repo, &["--worker", "go"]);
    let (exit, rows, summary) = s.wait(&["kept"], &["--timeout", "60"]);
    assert_eq!(exit, 0, "{rows:?} {summary}");
    let accepted = accept(&s, "kept", &["--review", "good"]);
    assert_eq!(accepted["state"], "accepted", "{accepted}");

    let brief = s.brief("kept-again", "start it again");
    let (code, _, err) =
        s.out(&["task", "start", "kept", "--brief", &brief, "--in", repo.to_str().unwrap(), "--worker", "go"]);
    assert_ne!(code, 0, "an accepted name is still refused");
    let error: Value = serde_json::from_str(err.trim()).unwrap();
    assert_eq!(error["error"]["kind"], "name_taken", "{error}");
    assert_eq!(error["error"]["hint"], "pick another name; an accepted or unfinished task keeps its name", "{error}");
}
