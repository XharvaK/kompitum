//! Work brought in by hand is recorded as accepted.

use super::*;

const REPORT: &str = "the worker's report\n\n## Gaps\nNone.\n";

/// Start one task, let it finish, and return its wake line.
fn run_one(s: &Session, repo: &Path, name: &str, mut actions: Vec<Value>, extra: &[&str]) -> Value {
    actions.extend(say(&stream(&format!("ses_{name}"), REPORT)));
    actions.push(json!({"exit": 0}));
    let script = s.script(name, json!([actions]));
    s.config(&script, "");
    let started = s.start(name, "do the work", repo, extra);
    assert_eq!(started["state"], "running", "{started}");
    let (_, rows, summary) = s.wait(&[name], &["--timeout", "60"]);
    assert_eq!(rows[0]["state"], "done", "{rows:?} {summary}");
    rows[0].clone()
}

fn refused(s: &Session, args: &[&str]) -> Value {
    let (_, _, err) = s.out(args);
    serde_json::from_str(err.trim()).unwrap_or_else(|e| panic!("{args:?}: {e}: {err}"))
}

#[test]
fn a_hand_committed_task_is_recorded_with_no_new_commit() {
    let s = Session::new();
    let repo = s.repo();
    run_one(&s, &repo, "hand-1", vec![json!({"write": ["made.txt", "hello"]})], &["--worker", "go"]);
    let worktree = worktree_of(&s, "hand-1");
    std::fs::copy(worktree.join("made.txt"), repo.join("made.txt")).unwrap();
    git(&repo, &["add", "made.txt"]);
    git(&repo, &["commit", "-q", "-m", "landed by hand"]);
    let head = git(&repo, &["rev-parse", "HEAD"]);
    let recorded = accept(&s, "hand-1", &["--recorded", "--review", "landed by hand"]);
    assert_eq!(recorded["state"], "accepted", "{recorded}");
    assert!(recorded["commit"].is_null(), "nothing was committed: {recorded}");
    assert_eq!(recorded["files"], json!(["made.txt"]), "{recorded}");
    assert_eq!(git(&repo, &["rev-parse", "HEAD"]), head, "the repository has no new commit");
    assert!(git(&repo, &["status", "--porcelain"]).trim().is_empty(), "the repository is clean");
    assert!(!worktree.exists(), "the worktree is gone");
    let ledger = s.json(&["ledger", "--json", "--task", "hand-1"]);
    let rows = ledger["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "{ledger}");
    assert_eq!(rows[0]["outcome"], "accepted", "{ledger}");
    assert_eq!(rows[0]["review"], "landed by hand", "{ledger}");
}

#[test]
fn a_dirty_file_refuses_a_recorded_accept_and_is_named() {
    let s = Session::new();
    let repo = s.repo();
    run_one(&s, &repo, "hand-2", vec![json!({"write": ["a.txt", "worker\n"]})], &["--worker", "go"]);
    std::fs::write(repo.join("a.txt"), "mine\n").unwrap();
    let error = refused(&s, &["task", "accept", "hand-2", "--recorded"]);
    assert_eq!(error["error"]["kind"], "files_dirty", "{error}");
    assert!(error["error"]["message"].as_str().unwrap().contains("a.txt"), "{error}");
}

#[test]
fn recorded_with_no_commit_is_refused() {
    let s = Session::new();
    let repo = s.repo();
    run_one(&s, &repo, "hand-3", vec![json!({"write": ["made.txt", "hello"]})], &["--worker", "go"]);
    let error = refused(&s, &["task", "accept", "hand-3", "--recorded", "--no-commit"]);
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("--recorded applies nothing, so --no-commit has no meaning"),
        "{error}"
    );
}

#[test]
fn a_batch_records_one_part_or_every_open_part() {
    let s = Session::new();
    let repo = s.repo();
    let parts: [(&str, &str, &[&str]); 2] = [("a", "do a", &["a.txt"]), ("b", "do b", &["b.txt"])];
    let mut actions = vec![json!({"write": ["a.txt", "A"]}), json!({"write": ["b.txt", "B"]})];
    actions.extend(say(&stream("ses_hand-batch", REPORT)));
    actions.push(json!({"exit": 0}));
    let script = s.script("hand-batch", json!([actions]));
    s.config(&script, "");
    let started = s.start_plan("hand-batch", &parts, None, &repo, &["--worker", "go"]);
    assert_eq!(started["state"], "running", "{started}");
    let (_, rows, summary) = s.wait(&["hand-batch"], &["--timeout", "60"]);
    assert_eq!(rows[0]["state"], "done", "{rows:?} {summary}");
    let worktree = worktree_of(&s, "hand-batch");
    for file in ["a.txt", "b.txt"] {
        std::fs::copy(worktree.join(file), repo.join(file)).unwrap();
    }
    git(&repo, &["add", "a.txt", "b.txt"]);
    git(&repo, &["commit", "-q", "-m", "landed by hand"]);
    let first = accept(&s, "hand-batch", &["--recorded", "--part", "a"]);
    assert_eq!(first["state"], "done", "part a alone leaves the task open: {first}");
    assert_eq!(first["files"], json!(["a.txt"]), "{first}");
    assert_eq!(s.task("hand-batch")["task"]["part_states"], json!(["accepted", "open"]));
    let last = accept(&s, "hand-batch", &["--recorded"]);
    assert_eq!(last["state"], "accepted", "{last}");
    assert_eq!(last["files"], json!(["b.txt"]), "{last}");
    assert!(!worktree.exists(), "the last part settles the task");
    let ledger = s.json(&["ledger", "--json", "--task", "hand-batch"]);
    assert_eq!(ledger["rows"].as_array().unwrap().len(), 1, "one row for the whole batch: {ledger}");
}
