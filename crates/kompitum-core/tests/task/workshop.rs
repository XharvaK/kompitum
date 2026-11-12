//! Tasks that share one workshop: disjoint scopes, overlaps, discards, strays, a task without a scope
//! and the one-writer rule.

use super::*;

/// The repository's checks file, committed as a repository commits its own.
fn commit_checks(repo: &Path, checks: Value) {
    let dir = repo.join(".kompitum");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("checks.json"), serde_json::to_vec_pretty(&checks).unwrap()).unwrap();
    git(repo, &["-c", "user.name=t", "-c", "user.email=t@t", "add", "."]);
    git(repo, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "checks"]);
}

/// One turn: the writes, then a good report.
fn turn(session: &str, writes: &[(&str, &str)]) -> Vec<Value> {
    let mut actions: Vec<Value> = writes.iter().map(|(path, text)| json!({"write": [path, text]})).collect();
    actions.extend(say(&stream(session, "done")));
    actions
}

/// How many turns the script has handed out so far.
fn handed_out(script: &Path) -> String {
    std::fs::read_to_string(format!("{}.count", script.display())).unwrap_or_default().trim().to_string()
}

/// Waits until the first worker has taken its turn, so the next worker takes the one after it.
fn first_worker_took_its_turn(script: &Path) {
    wait_until("the first worker takes its turn", || handed_out(script) == "1");
}

/// The paths a task's view lists as changed.
fn changed_paths(view: &Value) -> Vec<String> {
    view["task"]["changed"].as_array().unwrap().iter().map(|file| file["path"].as_str().unwrap().to_string()).collect()
}

fn warned(view: &Value, words: &str) -> bool {
    view["task"]["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|warning| warning.as_str().is_some_and(|w| w.contains(words)))
}

#[test]
fn tasks_with_disjoint_scopes_share_one_workshop() {
    let s = Session::new();
    let repo = s.repo();
    commit_checks(&repo, json!({"workshop": true}));
    let script =
        s.script("share", json!([turn("ses_a", &[("src/a.rs", "a\n")]), turn("ses_b", &[("docs/b.md", "b\n")])]));
    s.config(&script, "");
    s.start("a", "write a", &repo, &["--worker", "go", "--scope", "src/**"]);
    first_worker_took_its_turn(&script);
    s.start("b", "write b", &repo, &["--worker", "go", "--scope", "docs/**"]);
    let (exit, rows, _) = s.wait(&["a", "b"], &["--timeout", "60"]);
    assert_eq!(exit, 0, "{rows:?}");
    let a = s.task("a");
    let b = s.task("b");
    assert_eq!(a["task"]["shared"], true, "{a}");
    assert_eq!(b["task"]["shared"], true, "{b}");
    assert_eq!(a["task"]["worktree"], b["task"]["worktree"], "both work in one folder");
    let workshop = PathBuf::from(a["task"]["worktree"].as_str().unwrap());
    assert!(workshop.to_string_lossy().contains("workshops"), "{}", workshop.display());
    assert_eq!(changed_paths(&a), ["src/a.rs"], "{a}");
    assert_eq!(changed_paths(&b), ["docs/b.md"], "{b}");

    let accepted = s.json(&["task", "accept", "a", "--review", "ok"]);
    assert_eq!(accepted["removed"], true, "{accepted}");
    let last = git(&repo, &["show", "--stat", "--format=", "HEAD"]);
    assert!(last.contains("src/a.rs") && !last.contains("docs/b.md"), "{last}");
    assert!(workshop.join("docs").join("b.md").exists(), "b's file stays in the workshop");

    let (code, review, _) = s.out(&["task", "review", "b"]);
    assert_eq!(code, 0, "{review}");
    assert!(review.contains("docs/b.md") && !review.contains("src/a.rs"), "{review}");

    let accepted_b = s.json(&["task", "accept", "b", "--review", "ok"]);
    assert_eq!(accepted_b["removed"], true, "{accepted_b}");
    assert_eq!(git(&workshop, &["status", "--porcelain"]), "", "the workshop is clean");
    assert_eq!(git(&workshop, &["rev-parse", "HEAD"]), git(&repo, &["rev-parse", "HEAD"]));
}

#[test]
fn an_overlapping_task_waits_until_the_first_is_accepted() {
    let s = Session::new();
    let repo = s.repo();
    commit_checks(&repo, json!({"workshop": true}));
    let script =
        s.script("overlap", json!([turn("ses_a", &[("src/a.rs", "a\n")]), turn("ses_b", &[("src/b.rs", "b\n")])]));
    s.config(&script, "");
    s.start("a", "write a", &repo, &["--worker", "go", "--scope", "src/**"]);
    let (exit, rows, _) = s.wait(&["a"], &["--timeout", "60"]);
    assert_eq!(exit, 0, "{rows:?}");
    let waiting = "waiting for a: its files in the workshop are not accepted or discarded yet";
    let started = s.start("b", "write b", &repo, &["--worker", "go", "--scope", "src/b.rs"]);
    assert_eq!(started["state"], "queued", "{started}");
    wait_until("b names the task it waits for", || s.task("b")["task"]["waiting"] == waiting);

    s.json(&["task", "accept", "a", "--review", "ok"]);
    let (exit, rows, _) = s.wait(&["b"], &["--timeout", "60"]);
    assert_eq!(exit, 0, "{rows:?}");
    assert_eq!(rows[0]["state"], "done", "{rows:?}");
}

#[test]
fn discard_puts_the_files_back_and_keeps_the_workshop() {
    let s = Session::new();
    let repo = s.repo();
    // Windows' own line endings would make the committed text differ from the bytes the test compares.
    git(&repo, &["config", "core.autocrlf", "false"]);
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(repo.join("src").join("keep.rs"), "keep committed\n").unwrap();
    commit_checks(&repo, json!({"workshop": true}));
    let mut actions = vec![json!({"write": ["src/keep.rs", "changed\n"]}), json!({"write": ["src/new.rs", "new\n"]})];
    actions.extend(say(&stream("ses_d", "done")));
    let script = s.script("discard-shared", json!([actions]));
    s.config(&script, "");
    s.start("d", "rewrite", &repo, &["--worker", "go", "--scope", "src/**"]);
    let (exit, rows, _) = s.wait(&["d"], &["--timeout", "60"]);
    assert_eq!(exit, 0, "{rows:?}");
    let workshop = PathBuf::from(s.task("d")["task"]["worktree"].as_str().unwrap());
    assert_eq!(std::fs::read_to_string(workshop.join("src").join("keep.rs")).unwrap(), "changed\n");

    let discarded = s.json(&["task", "discard", "d"]);
    assert_eq!(discarded["removed"], true, "{discarded}");
    assert!(workshop.is_dir(), "the workshop is never removed");
    assert_eq!(std::fs::read_to_string(workshop.join("src").join("keep.rs")).unwrap(), "keep committed\n");
    assert!(!workshop.join("src").join("new.rs").exists(), "the new file is gone");
    assert_eq!(git(&workshop, &["status", "--porcelain"]), "", "the workshop is clean");
}

#[test]
fn a_write_outside_the_scope_is_a_stray_not_the_tasks_change() {
    let s = Session::new();
    let repo = s.repo();
    commit_checks(&repo, json!({"workshop": true}));
    let script = s.script("stray", json!([turn("ses_s", &[("src/a.rs", "a\n"), ("other.txt", "x\n")])]));
    s.config(&script, "");
    s.start("s1", "stray", &repo, &["--worker", "go", "--scope", "src/**"]);
    let (_, rows, _) = s.wait(&["s1"], &["--timeout", "60"]);
    assert_eq!(rows[0]["state"], "done", "{rows:?}");
    let view = s.task("s1");
    assert_eq!(changed_paths(&view), ["src/a.rs"], "{view}");
    assert!(warned(&view, "outside every task's scope: other.txt"), "{view}");
}

#[test]
fn a_task_without_a_scope_gets_a_worktree_of_its_own() {
    let s = Session::new();
    let repo = s.repo();
    commit_checks(&repo, json!({"workshop": true}));
    let script = s.script("no-scope", json!([turn("ses_n", &[("notes.txt", "n\n")])]));
    s.config(&script, "");
    s.start("n1", "no scope", &repo, &["--worker", "go"]);
    let (exit, rows, _) = s.wait(&["n1"], &["--timeout", "60"]);
    assert_eq!(exit, 0, "{rows:?}");
    let view = s.task("n1");
    assert_eq!(view["task"]["shared"], false, "{view}");
    assert!(view["task"]["worktree"].as_str().unwrap().contains("worktrees"), "{view}");
    assert!(warned(&view, "gets a worktree of its own"), "{view}");
}

#[test]
fn one_writer_tasks_take_turns() {
    let s = Session::new();
    let repo = s.repo();
    commit_checks(&repo, json!({"workshop": {"one_writer": ["src/**"]}}));
    let mut first = vec![json!({"sleep_ms": 6000}), json!({"write": ["src/a/x.rs", "a\n"]})];
    first.extend(say(&stream("ses_w1", "done")));
    let script = s.script("one-writer", json!([first, turn("ses_w2", &[("src/b/y.rs", "b\n")])]));
    s.config(&script, "");
    s.start("wa", "writer a", &repo, &["--worker", "go", "--scope", "src/a/**"]);
    first_worker_took_its_turn(&script);
    let started = s.start("wb", "writer b", &repo, &["--worker", "go", "--scope", "src/b/**"]);
    assert_eq!(started["state"], "queued", "{started}");
    wait_until("wb waits for the one writer", || {
        s.task("wb")["task"]["waiting"].as_str().is_some_and(|words| words.contains("one writer at a time"))
    });
    let (exit, rows, _) = s.wait(&["wa", "wb"], &["--timeout", "60"]);
    assert_eq!(exit, 0, "{rows:?}");
    for row in &rows {
        assert_eq!(row["state"], "done", "{rows:?}");
    }
}
