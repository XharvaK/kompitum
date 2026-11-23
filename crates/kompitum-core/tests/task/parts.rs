//! A batch's parts end to end: one coordinator turn, then accept or discard by part.

use super::handoff::go_good;
use super::*;

const REPORT: &str = "the worker's report\n\n## Gaps\nNone.\n";

fn two_parts() -> [(&'static str, &'static str, &'static [&'static str]); 2] {
    [("a", "do a", &["a.txt"]), ("b", "do b", &["b.txt"])]
}

/// One batch turn that writes what each part asks for, then reports and ends.
fn run_batch(
    s: &Session,
    repo: &Path,
    name: &str,
    parts: &[(&str, &str, &[&str])],
    writes: Vec<Value>,
    extra: &[&str],
) -> Value {
    let mut actions = writes;
    actions.extend(say(&stream(&format!("ses_{name}"), REPORT)));
    actions.push(json!({"exit": 0}));
    let script = s.script(name, json!([actions]));
    s.config(&script, "");
    let started = s.start_plan(name, parts, None, repo, extra);
    assert_eq!(started["state"], "running", "{started}");
    let (_, rows, summary) = s.wait(&[name], &["--timeout", "60"]);
    assert_eq!(rows[0]["state"], "done", "{rows:?} {summary}");
    rows[0].clone()
}

#[test]
fn a_batchs_prompt_carries_the_coordinator_section_before_the_footer() {
    let s = Session::new();
    let repo = s.repo();
    let kept = s.root.join("batch-prompt.md");
    let script = s.script("batch-prompt", json!([go_good(Some(&kept))]));
    s.config(&script, "");
    let started = s.start_plan("batch-prompt", &two_parts(), None, &repo, &["--worker", "go"]);
    assert_eq!(started["state"], "running", "{started}");
    let (_, rows, summary) = s.wait(&["batch-prompt"], &["--timeout", "60"]);
    assert_eq!(rows[0]["state"], "done", "{rows:?} {summary}");
    let prompt = std::fs::read_to_string(&kept).unwrap();
    let shared = prompt.find("The parts below are this batch's work.").unwrap();
    let section = prompt.find("# This task is a batch").unwrap();
    let footer = prompt.find("# Limits on this task").unwrap();
    assert!(shared < section && section < footer, "the section sits after the brief and before the footer: {prompt}");
    let folder = s.state().join("tasks").join("batch-prompt");
    for part in ["a", "b"] {
        let path = folder.join(format!("part-{part}.md")).display().to_string().replace('\\', "/");
        assert!(prompt.contains(&format!("brief `{path}`")), "the section names part {part}'s file: {prompt}");
        assert!(folder.join(format!("part-{part}.md")).is_file(), "part {part}'s brief is beside brief.md");
    }
}

#[test]
fn accepting_one_part_commits_only_its_files_and_keeps_the_task() {
    let s = Session::new();
    let repo = s.repo();
    run_batch(
        &s,
        &repo,
        "batch-one",
        &two_parts(),
        vec![json!({"write": ["a.txt", "A"]}), json!({"write": ["b.txt", "B"]})],
        &["--worker", "go"],
    );
    let worktree = worktree_of(&s, "batch-one");
    let accepted = accept(&s, "batch-one", &["--part", "a", "--message", "first"]);
    assert_eq!(accepted["state"], "done", "{accepted}");
    assert_eq!(accepted["files"], json!(["a.txt"]), "{accepted}");
    assert!(accepted["commit"].as_str().is_some_and(|commit| commit.len() >= 40), "{accepted}");
    assert_eq!(accepted["removed"], false, "a part's accept keeps the worktree: {accepted}");
    assert_eq!(git(&repo, &["show", "--name-only", "--format=", "HEAD"]).trim(), "a.txt");
    assert_eq!(git(&repo, &["log", "-1", "--format=%s"]).trim(), "first");
    assert!(worktree.exists(), "the worktree waits for part b");
    assert_eq!(std::fs::read_to_string(worktree.join("b.txt")).unwrap(), "B", "b's work stays changed");
    let view = s.task("batch-one");
    assert_eq!(view["task"]["part_states"], json!(["accepted", "open"]), "{view}");
}

#[test]
fn accepting_the_last_part_settles_the_task_with_one_ledger_row() {
    let s = Session::new();
    let repo = s.repo();
    run_batch(
        &s,
        &repo,
        "batch-last",
        &two_parts(),
        vec![json!({"write": ["a.txt", "A"]}), json!({"write": ["b.txt", "B"]})],
        &["--worker", "go"],
    );
    let worktree = worktree_of(&s, "batch-last");
    let first = accept(&s, "batch-last", &["--part", "a", "--message", "first"]);
    assert_eq!(first["state"], "done", "{first}");
    let last = accept(&s, "batch-last", &["--part", "b", "--message", "second"]);
    assert_eq!(last["state"], "accepted", "{last}");
    assert_eq!(last["files"], json!(["b.txt"]), "{last}");
    assert_eq!(last["removed"], true, "{last}");
    assert_eq!(git(&repo, &["log", "-1", "--format=%s"]).trim(), "second");
    assert!(!worktree.exists(), "the worktree is gone");
    assert!(git(&repo, &["branch", "--list", "task/batch-last"]).trim().is_empty(), "the branch is gone");
    let ledger = s.json(&["ledger", "--json", "--task", "batch-last"]);
    let rows = ledger["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "one row for the whole task: {ledger}");
    assert_eq!(rows[0]["outcome"], "accepted", "{ledger}");
}

#[test]
fn discarding_a_part_restores_its_files_to_the_base_and_keeps_the_task() {
    let s = Session::new();
    let repo = s.repo();
    std::fs::write(repo.join("b.txt"), "base\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "b starts here"]);
    run_batch(
        &s,
        &repo,
        "batch-discard",
        &two_parts(),
        vec![json!({"write": ["a.txt", "A"]}), json!({"write": ["b.txt", "changed"]})],
        &["--worker", "go"],
    );
    let worktree = worktree_of(&s, "batch-discard");
    let discarded = s.json(&["task", "discard", "batch-discard", "--part", "b", "--review", "not wanted"]);
    assert_eq!(discarded["state"], "done", "{discarded}");
    assert_eq!(discarded["removed"], false, "{discarded}");
    assert!(worktree.exists(), "the task waits for part a");
    let restored = std::fs::read_to_string(worktree.join("b.txt")).unwrap().replace("\r\n", "\n");
    assert_eq!(restored, "base\n", "b is back at the base");
    let accepted = accept(&s, "batch-discard", &["--part", "a", "--message", "take a"]);
    assert_eq!(accepted["state"], "accepted", "{accepted}");
    assert_eq!(accepted["removed"], true, "{accepted}");
    assert!(!worktree.exists(), "accepting the last open part settles the task");
}

#[test]
fn accepting_every_part_makes_one_commit_each_in_part_order() {
    let s = Session::new();
    let repo = s.repo();
    run_batch(
        &s,
        &repo,
        "batch-all",
        &two_parts(),
        vec![json!({"write": ["a.txt", "A"]}), json!({"write": ["b.txt", "B"]})],
        &["--worker", "go"],
    );
    let worktree = worktree_of(&s, "batch-all");
    let accepted = accept(&s, "batch-all", &["--message", "batch"]);
    assert_eq!(accepted["state"], "accepted", "{accepted}");
    let log = git(&repo, &["log", "-2", "--format=%s"]);
    let messages: Vec<&str> = log.lines().collect();
    assert_eq!(messages, ["batch (part b)", "batch (part a)"], "one commit per part, in order: {log}");
    assert_eq!(git(&repo, &["show", "--name-only", "--format=", "HEAD"]).trim(), "b.txt");
    assert_eq!(git(&repo, &["show", "--name-only", "--format=", "HEAD~1"]).trim(), "a.txt");
    assert!(!worktree.exists(), "the last part settles the task");
}

#[test]
fn a_part_without_changes_is_accepted_with_no_commit() {
    let s = Session::new();
    let repo = s.repo();
    run_batch(&s, &repo, "batch-empty", &two_parts(), vec![json!({"write": ["b.txt", "B"]})], &["--worker", "go"]);
    let head = git(&repo, &["rev-parse", "HEAD"]);
    let accepted = accept(&s, "batch-empty", &["--part", "a", "--message", "nothing to do"]);
    assert_eq!(accepted["state"], "done", "{accepted}");
    assert!(accepted["commit"].is_null(), "no commit for an unchanged part: {accepted}");
    assert_eq!(git(&repo, &["rev-parse", "HEAD"]), head, "the repository has no new commit");
    let note = accepted["note"].as_str().unwrap_or_default();
    assert!(note.contains("no changes") && note.contains("accepted with no commit"), "{accepted}");
    let view = s.task("batch-empty");
    assert_eq!(view["task"]["part_states"], json!(["accepted", "open"]), "{view}");
}

#[test]
fn a_failed_commit_on_a_part_keeps_the_earlier_commit_and_the_part_open() {
    let s = Session::new();
    s.env("GIT_CONFIG_COUNT", "0");
    let repo = s.repo();
    std::fs::write(repo.join("b.txt"), "base\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "b starts here"]);
    run_batch(
        &s,
        &repo,
        "batch-fail",
        &two_parts(),
        vec![json!({"write": ["a.txt", "A"]}), json!({"write": ["b.txt", "B"]})],
        &["--worker", "go"],
    );
    one_hook(&repo, "pre-commit", "if [ -f .git/reject ]; then\n  echo 'b is not wanted' >&2\n  exit 1\nfi");
    let first = accept(&s, "batch-fail", &["--part", "a", "--message", "first"]);
    assert!(first["commit"].as_str().is_some(), "{first}");
    std::fs::write(repo.join(".git").join("reject"), b"").unwrap();
    let (code, _, err) = s.out(&["task", "accept", "batch-fail", "--part", "b", "--message", "second"]);
    assert_ne!(code, 0, "b's commit fails: {err}");
    let error: Value = serde_json::from_str(err.trim()).unwrap();
    let message = error["error"]["message"].as_str().unwrap_or_default();
    assert!(message.contains("part b"), "the error names b: {error}");
    assert_eq!(git(&repo, &["log", "-1", "--format=%s"]).trim(), "first", "a's commit stays");
    assert_eq!(git(&repo, &["show", "--name-only", "--format=", "HEAD"]).trim(), "a.txt");
    let view = s.task("batch-fail");
    assert_eq!(view["task"]["part_states"], json!(["accepted", "open"]), "{view}");
}
