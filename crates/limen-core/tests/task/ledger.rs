//! The ledger, the orchestrator's own usage, and the sweep.

use super::*;

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64
}

fn names(result: &Value, key: &str) -> Vec<String> {
    result[key].as_array().into_iter().flatten().filter_map(Value::as_str).map(String::from).collect()
}

/// Stop the core, then set each record's end far enough back for a sweep to see it.
fn age_records(s: &Session, tasks: &[&str], days: u64) {
    s.out(&["server", "stop"]);
    for task in tasks {
        let path = s.state().join("tasks").join(task).join("record.json");
        let mut record: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        record["ended_ms"] = json!(now_ms() - days * 86_400_000);
        std::fs::write(&path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
    }
}

fn ledger_line(s: &Session) -> Value {
    let text = std::fs::read_to_string(s.state().join("ledger.jsonl")).unwrap();
    serde_json::from_str(text.lines().next().unwrap()).unwrap()
}

fn append_row(s: &Session, row: &Value) {
    let path = s.state().join("ledger.jsonl");
    let mut text = std::fs::read_to_string(&path).unwrap();
    text.push_str(&serde_json::to_string(row).unwrap());
    text.push('\n');
    std::fs::write(&path, text).unwrap();
}

#[test]
fn a_discard_writes_a_row() {
    let s = Session::new();
    let repo = s.repo();
    let script = s.script("row", json!([say(&stream("ses_row", "finished"))]));
    s.config(&script, "");
    s.start("fix-1", "write something", &repo, &["--worker", "go"]);
    let (exit, rows, _) = s.wait(&["fix-1"], &["--timeout", "30"]);
    assert_eq!(exit, 0, "{rows:?}");
    let discarded = s.json(&["task", "discard", "fix-1", "--review", "the work is sound"]);
    assert_eq!(discarded["state"], "discarded", "{discarded}");

    let ledger = s.json(&["ledger", "--json"]);
    let rows = ledger["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "{ledger}");
    assert_eq!(rows[0]["task"], "fix-1");
    assert_eq!(rows[0]["outcome"], "discarded");
    assert_eq!(rows[0]["review"], "the work is sound");
    assert_eq!(rows[0]["role"], "worker");
    assert_eq!(rows[0]["workers"], json!(["go"]));
    assert_eq!(rows[0]["turns"][0]["role"], "worker");
    let (_, text, _) = s.out(&["ledger"]);
    assert!(text.contains("discarded"), "{text}");
    assert!(text.contains("the work is sound"), "{text}");

    let again = s.json(&["task", "discard", "fix-1"]);
    assert_eq!(again["state"], "discarded");
    let ledger = s.json(&["ledger", "--json"]);
    assert_eq!(ledger["rows"].as_array().unwrap().len(), 1, "a second discard writes no second row");
}

#[test]
fn a_damaged_line_is_named_and_the_rest_still_read() {
    let s = Session::new();
    let repo = s.repo();
    let script = s.script("damaged", json!([say(&stream("ses_damaged", "finished"))]));
    s.config(&script, "");
    s.start("fix-2", "write something", &repo, &["--worker", "go"]);
    let (exit, rows, _) = s.wait(&["fix-2"], &["--timeout", "30"]);
    assert_eq!(exit, 0, "{rows:?}");
    s.json(&["task", "discard", "fix-2"]);
    let path = s.state().join("ledger.jsonl");
    let mut text = std::fs::read_to_string(&path).unwrap();
    text.push_str("this is not a ledger row\n");
    std::fs::write(&path, text).unwrap();

    let ledger = s.json(&["ledger", "--json"]);
    assert_eq!(ledger["rows"].as_array().unwrap().len(), 1, "the good row still reads: {ledger}");
    let problems = ledger["problems"].as_array().unwrap();
    assert!(
        problems.iter().any(|problem| problem.as_str().unwrap_or_default().contains("line 2")),
        "the damaged line is named with its number: {problems:?}"
    );
}

#[test]
fn the_ledger_filters_by_task_and_takes_the_newest() {
    let s = Session::new();
    let repo = s.repo();
    let script = s.script("filter", json!([say(&stream("ses_filter", "finished"))]));
    s.config(&script, "");
    s.start("fix-1", "write something", &repo, &["--worker", "go"]);
    let (exit, rows, _) = s.wait(&["fix-1"], &["--timeout", "30"]);
    assert_eq!(exit, 0, "{rows:?}");
    s.json(&["task", "discard", "fix-1"]);
    let first = ledger_line(&s);
    for (task, at) in [("fix-2", 1_000u64), ("fix-3", 2_000u64)] {
        let mut row = first.clone();
        row["task"] = json!(task);
        row["at_ms"] = json!(at);
        append_row(&s, &row);
    }

    let newest = s.json(&["ledger", "--json", "--last", "2"]);
    let tasks: Vec<&str> = newest["rows"].as_array().unwrap().iter().filter_map(|row| row["task"].as_str()).collect();
    assert_eq!(tasks, ["fix-2", "fix-3"], "the newest two, oldest first: {newest}");

    let one = s.json(&["ledger", "--json", "--task", "fix-1"]);
    let rows = one["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "{one}");
    assert_eq!(rows[0]["task"], "fix-1");
    assert_eq!(one["totals"]["cost_usd"], rows[0]["usage"]["cost_usd"], "the totals are of the shown rows");

    let mut newer = first;
    newer["at_ms"] = json!(3_000u64);
    append_row(&s, &newer);
    let newest_one = s.json(&["ledger", "--json", "--task", "fix-1", "--last", "1"]);
    let rows = newest_one["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "the last applies with a task too: {newest_one}");
    assert_eq!(rows[0]["at_ms"], 3_000, "{newest_one}");
}

#[test]
fn the_parent_line_shows_for_a_task() {
    let s = Session::new();
    let repo = s.repo();
    let claude = s.root.join("claude");
    s.env("CLAUDE_CONFIG_DIR", &claude);
    let script = s.script("parent", json!([say(&stream("ses_parent", "finished"))]));
    s.config(&script, "");
    s.start("fix-5", "write something", &repo, &["--worker", "go"]);
    let (exit, rows, _) = s.wait(&["fix-5"], &["--timeout", "30"]);
    assert_eq!(exit, 0, "{rows:?}");
    s.json(&["task", "discard", "fix-5"]);

    let folder = claude.join("projects").join("repo");
    std::fs::create_dir_all(&folder).unwrap();
    let transcript = folder.join("parent.jsonl");
    let lines = [
        json!({"type": "assistant", "message": {"id": "m1", "role": "assistant",
            "content": [{"type": "tool_use", "name": "Bash", "input": {"command": "lc task start fix-5 --brief b.md"}}],
            "usage": {"input_tokens": 10, "cache_read_input_tokens": 100000, "output_tokens": 5000}}}),
        json!({"type": "assistant", "message": {"id": "m2", "role": "assistant",
            "content": [{"type": "tool_use", "name": "Bash", "input": {"command": "lc task accept fix-5"}}],
            "usage": {"input_tokens": 2, "cache_read_input_tokens": 200000, "output_tokens": 7000}}}),
    ];
    let text: String = lines.iter().map(|line| format!("{line}\n")).collect();
    std::fs::write(&transcript, text).unwrap();

    let ledger = s.json(&["ledger", "--json", "--task", "fix-5"]);
    assert_eq!(ledger["parent"]["requests"], 2, "{ledger}");
    assert_eq!(ledger["parent"]["closed"], true, "{ledger}");
    let (_, text, _) = s.out(&["ledger", "--task", "fix-5"]);
    assert!(text.contains("Parent: 2 requests, 300K cache reads, 12K output, peak 200K."), "{text}");

    let nobody = folder.join("nobody.jsonl");
    let line = json!({"type": "assistant", "message": {"id": "m1", "role": "assistant",
        "content": [{"type": "tool_use", "name": "Bash", "input": {"command": "lc task start nobody-ran-this"}}],
        "usage": {"input_tokens": 10}}});
    std::fs::write(&nobody, format!("{line}\n")).unwrap();
    let none = s.json(&["ledger", "--json", "--task", "nobody-ran-this"]);
    assert!(none["parent"].is_null(), "no row and no record, so no transcript is searched: {none}");
    assert!(!none["table"].as_str().unwrap().contains("Parent:"), "{none}");
    std::fs::remove_file(&nobody).unwrap();

    std::fs::remove_file(&transcript).unwrap();
    let (_, text, _) = s.out(&["ledger", "--task", "fix-5"]);
    assert!(!text.contains("Parent:"), "no transcript, no parent line: {text}");
}

#[test]
fn the_sweep_removes_an_old_discarded_task_and_frees_its_name() {
    let s = Session::new();
    let repo = s.repo();
    let script = s.script("old", json!([say(&stream("ses_old", "finished"))]));
    s.config(&script, "");
    s.start("old", "sweep me", &repo, &["--worker", "go"]);
    let (exit, rows, _) = s.wait(&["old"], &["--timeout", "30"]);
    assert_eq!(exit, 0, "{rows:?}");
    s.json(&["task", "discard", "old", "--review", "stale"]);
    let folder = s.state().join("tasks").join("old");
    assert!(folder.exists(), "the discarded task still has its folder");
    let patches = s.state().join("patches");
    std::fs::create_dir_all(&patches).unwrap();
    let patch = patches.join("old.patch");
    std::fs::write(&patch, "a kept patch").unwrap();
    age_records(&s, &["old"], 15);

    // A core sweeps by itself at its start with the default days.
    let (code, text, err) = s.out(&["task", "ls", "--all"]);
    assert_eq!(code, 0, "{text}{err}");
    wait_until("the old task's folder went", || !folder.exists());
    assert!(!patch.exists(), "its kept patch went too");
    let ledger = s.json(&["ledger", "--json", "--task", "old"]);
    assert_eq!(ledger["rows"].as_array().unwrap().len(), 1, "the row stays: {ledger}");

    let brief = s.brief("again", "use the name again");
    let repo_text = repo.display().to_string();
    let (code, text, err) = s.out(&["task", "start", "old", "--brief", &brief, "--in", &repo_text]);
    assert_eq!(code, 0, "the name is free again: {text}{err}");
    s.json(&["task", "cancel", "old"]);
}

#[test]
fn a_dry_run_removes_nothing() {
    let s = Session::new();
    let repo = s.repo();
    let script = s.script("dry", json!([say(&stream("ses_dry", "finished"))]));
    s.config(&script, "");
    s.start("dry", "sweep me", &repo, &["--worker", "go"]);
    let (exit, rows, _) = s.wait(&["dry"], &["--timeout", "30"]);
    assert_eq!(exit, 0, "{rows:?}");
    s.json(&["task", "discard", "dry"]);
    let bad = vec![json!({"out": stream("ses_dry_bad", "never")[0]}), json!({"exit": 3})];
    let failed = s.script("dry-failed", json!([bad]));
    s.config(&failed, "");
    s.start("dry-bad", "fail me", &repo, &["--worker", "go"]);
    let (exit, rows, _) = s.wait(&["dry-bad"], &["--timeout", "30"]);
    assert_eq!(exit, 1, "{rows:?}");
    age_records(&s, &["dry", "dry-bad"], 5);
    // The core's own sweep uses 14 days, so only the command can act on 5.
    s.out(&["task", "ls", "--all"]);

    let swept = s.json(&["task", "sweep", "--days", "1", "--dry-run"]);
    assert_eq!(swept["dry_run"], true, "{swept}");
    assert!(names(&swept, "removed").contains(&"dry".to_string()), "{swept}");
    assert!(names(&swept, "discarded").contains(&"dry-bad".to_string()), "{swept}");
    assert!(swept["freed_bytes"].as_u64().unwrap() > 0, "a dry run counts what would go: {swept}");
    assert!(s.state().join("tasks").join("dry").exists(), "nothing was removed");
    assert!(s.state().join("tasks").join("dry-bad").exists(), "a failed task was not discarded");
    let ledger = s.json(&["ledger", "--json", "--task", "dry"]);
    assert_eq!(ledger["rows"].as_array().unwrap().len(), 1, "the row stays: {ledger}");
}

/// Stop the core and rewrite one record's state, worktree and end.
fn edit_record(s: &Session, task: &str, change: impl FnOnce(&mut Value)) {
    s.out(&["server", "stop"]);
    let path = s.state().join("tasks").join(task).join("record.json");
    let mut record: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    change(&mut record);
    std::fs::write(&path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
}

#[test]
fn a_task_whose_worktree_stayed_is_not_listed_as_removed() {
    let s = Session::new();
    let repo = s.repo();
    let script = s.script("stayed", json!([say(&stream("ses_stayed", "finished"))]));
    s.config(&script, "");
    s.start("stayed", "sweep me", &repo, &["--worker", "go"]);
    let (exit, rows, _) = s.wait(&["stayed"], &["--timeout", "30"]);
    assert_eq!(exit, 0, "{rows:?}");
    let outside = s.root.join("outside-worktree");
    std::fs::create_dir_all(&outside).unwrap();
    edit_record(&s, "stayed", |record| {
        record["state"] = json!("discarded");
        record["worktree"] = json!(outside.display().to_string());
        record["ended_ms"] = json!(now_ms() - 5 * 86_400_000);
    });
    s.out(&["task", "ls", "--all"]);

    let swept = s.json(&["task", "sweep", "--days", "1"]);
    assert!(
        names(&swept, "left").iter().any(|word| word.contains("outside-worktree")),
        "what stayed is named: {swept}"
    );
    assert!(!names(&swept, "removed").contains(&"stayed".to_string()), "it was not cleanly removed: {swept}");
    assert!(!s.state().join("tasks").join("stayed").exists(), "its folder went: {swept}");
    assert!(outside.exists(), "the worktree stays for a person");
    let _ = std::fs::remove_dir_all(outside);
}

#[test]
fn a_sweep_names_what_a_discard_left() {
    let s = Session::new();
    let repo = s.repo();
    let script = s.script("left", json!([say(&stream("ses_left", "finished"))]));
    s.config(&script, "");
    s.start("left", "fail me", &repo, &["--worker", "go"]);
    let (exit, rows, _) = s.wait(&["left"], &["--timeout", "30"]);
    assert_eq!(exit, 0, "{rows:?}");
    let outside = s.root.join("left-worktree");
    std::fs::create_dir_all(&outside).unwrap();
    edit_record(&s, "left", |record| {
        record["state"] = json!("failed");
        record["worktree"] = json!(outside.display().to_string());
        record["ended_ms"] = json!(now_ms() - 5 * 86_400_000);
    });
    s.out(&["task", "ls", "--all"]);

    let swept = s.json(&["task", "sweep", "--days", "1"]);
    assert!(names(&swept, "discarded").contains(&"left".to_string()), "{swept}");
    assert!(
        names(&swept, "left").iter().any(|word| word.contains("left-worktree")),
        "the discard's words reach the sweep: {swept}"
    );
    let _ = std::fs::remove_dir_all(outside);
}

#[test]
fn an_old_failed_task_is_discarded_and_a_done_one_stays() {
    let s = Session::new();
    let repo = s.repo();
    let bad = vec![json!({"out": stream("ses_bad", "never")[0]}), json!({"exit": 3})];
    let good = say(&stream("ses_good", "all done"));
    let script = s.script("failed", json!([bad, good]));
    s.config(&script, "");
    s.start("bad-1", "fail", &repo, &["--worker", "go"]);
    let (exit, rows, _) = s.wait(&["bad-1"], &["--timeout", "30"]);
    assert_eq!(exit, 1, "{rows:?}");
    assert_eq!(rows[0]["state"], "failed", "{rows:?}");
    s.start("done-1", "stay", &repo, &["--worker", "go"]);
    let (exit, rows, _) = s.wait(&["done-1"], &["--timeout", "30"]);
    assert_eq!(exit, 0, "{rows:?}");
    assert_eq!(rows[0]["state"], "done", "{rows:?}");
    age_records(&s, &["bad-1", "done-1"], 5);
    s.out(&["task", "ls", "--all"]);

    let swept = s.json(&["task", "sweep", "--days", "1"]);
    assert!(names(&swept, "discarded").contains(&"bad-1".to_string()), "{swept}");
    assert!(!names(&swept, "removed").contains(&"bad-1".to_string()), "its folder goes later: {swept}");
    assert!(!names(&swept, "removed").contains(&"done-1".to_string()), "done work stays: {swept}");
    let list = s.json(&["task", "ls", "--all"]);
    let done = list["tasks"].as_array().unwrap().iter().any(|task| task["task"] == "done-1" && task["state"] == "done");
    assert!(done, "a done task stays whatever its age: {list}");
    let ledger = s.json(&["ledger", "--json", "--task", "bad-1"]);
    let rows = ledger["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "the discard wrote its row: {ledger}");
    assert_eq!(rows[0]["outcome"], "discarded");
    assert!(rows[0]["review"].as_str().unwrap().contains("nobody came for it after 1 day"), "{ledger}");
    assert!(s.state().join("tasks").join("bad-1").exists(), "its folder goes on a later sweep");
}

#[test]
fn a_worktree_no_task_owns_is_only_named() {
    let s = Session::new();
    let stray = s.state().join("worktrees").join("stray");
    std::fs::create_dir_all(&stray).unwrap();
    std::fs::write(stray.join("work.txt"), "left here").unwrap();

    let swept = s.json(&["task", "sweep"]);
    let left = names(&swept, "left");
    assert!(left.iter().any(|word| word.contains("stray")), "the folder is named: {swept}");
    assert!(stray.join("work.txt").is_file(), "it is never removed");
}
