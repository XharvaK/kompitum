//! The review, accepting a task, and sending it back.

use super::*;

/// A Command Code turn that ends with a result line, as the stream parser needs.
fn cc_turn(session: &str, text: &str) -> Vec<Value> {
    let event = |value: Value| json!({"type": "event", "event": value}).to_string();
    let mut actions = say(&[
        event(json!({"type": "run_start", "sessionId": session})),
        event(json!({"type": "model_request_start", "model": "deepseek/deepseek-v4.1-flash"})),
        event(json!({
            "type": "model_request_end", "model": "deepseek/deepseek-v4.1-flash", "stopReason": "stop",
            "effort": "max",
            "usage": {"inputTokens": 10, "outputTokens": 5, "cacheReadTokens": 0, "cacheWriteTokens": 0}
        })),
        json!({
            "type": "result", "subtype": "success", "sessionId": session, "durationMs": 10,
            "finalText": text, "error": "",
            "usage": {"inputTokens": 10, "outputTokens": 5, "cacheReadTokens": 0, "cacheWriteTokens": 0}
        })
        .to_string(),
    ]);
    actions.push(json!({"exit": 0}));
    actions
}

fn report() -> &'static str {
    "the worker's report\n\n## Gaps\nNone.\n"
}

/// Start one task, let it finish, and return its wake line.
fn run_one(s: &Session, repo: &Path, name: &str, mut actions: Vec<Value>, extra: &[&str]) -> Value {
    actions.extend(say(&stream(&format!("ses_{name}"), report())));
    actions.push(json!({"exit": 0}));
    let script = s.script(name, json!([actions]));
    s.config(&script, "");
    let started = s.start(name, "do the work", repo, extra);
    assert_eq!(started["state"], "running", "{started}");
    let (_, rows, summary) = s.wait(&[name], &["--timeout", "60"]);
    assert_eq!(rows[0]["state"], "done", "{rows:?} {summary}");
    rows[0].clone()
}

fn accept(s: &Session, name: &str, extra: &[&str]) -> Value {
    let mut args = vec!["task", "accept", name];
    args.extend_from_slice(extra);
    s.json(&args)
}

fn refused(s: &Session, args: &[&str]) -> Value {
    let (_, _, err) = s.out(args);
    serde_json::from_str(err.trim()).unwrap_or_else(|e| panic!("{args:?}: {e}: {err}"))
}

fn review_fn(s: &Session, name: &str, extra: &[&str]) -> Value {
    let mut args = vec!["--json", "task", "review", name];
    args.extend_from_slice(extra);
    s.json(&args)
}

fn branch_of(s: &Session, name: &str) -> String {
    s.task(name)["task"]["branch"].as_str().unwrap().to_string()
}

fn worktree_of(s: &Session, name: &str) -> PathBuf {
    PathBuf::from(s.task(name)["task"]["worktree"].as_str().unwrap())
}

fn one_hook(repo: &Path, name: &str, body: &str) {
    let hook = repo.join(".git").join("hooks").join(name);
    std::fs::write(&hook, format!("#!/bin/sh\n{body}\n")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

#[test]
fn accept_commits_only_the_patchs_files() {
    let s = Session::new();
    let repo = s.repo();
    run_one(&s, &repo, "fix-1", vec![json!({"write": ["made.txt", "hello"]})], &["--worker", "go"]);
    let worktree = worktree_of(&s, "fix-1");
    std::fs::write(repo.join("a.txt"), "mine\n").unwrap();
    let accepted = accept(&s, "fix-1", &["--review", "looks good"]);
    assert_eq!(accepted["state"], "accepted", "{accepted}");
    assert_eq!(accepted["files"], json!(["made.txt"]), "{accepted}");
    assert!(accepted["commit"].as_str().is_some_and(|commit| commit.len() >= 40), "{accepted}");
    assert_eq!(accepted["removed"], true, "{accepted}");
    assert!(accepted["left"].as_array().unwrap().is_empty(), "{accepted}");
    assert_eq!(git(&repo, &["show", "--name-only", "--format=", "HEAD"]).trim(), "made.txt");
    assert_eq!(git(&repo, &["show", "HEAD:a.txt"]), "one\n", "the other change was not committed");
    assert_eq!(std::fs::read_to_string(repo.join("a.txt")).unwrap(), "mine\n", "and it is unchanged");
    assert!(
        git(&repo, &["status", "--porcelain", "--", "a.txt"]).contains("a.txt"),
        "the other change stays uncommitted"
    );
    assert!(!worktree.exists(), "the worktree is gone");
    assert!(git(&repo, &["branch", "--list", "task/fix-1"]).trim().is_empty(), "the branch is gone");
    assert!(Path::new(accepted["patch"].as_str().unwrap()).is_file(), "the patch is kept");
    assert!(s.task("fix-1")["task"]["worktree"].is_null());
}

#[test]
fn accept_commits_a_file_with_a_turkish_name() {
    let s = Session::new();
    let repo = s.repo();
    let name = "şarkı.txt";
    run_one(&s, &repo, "fix-turkish", vec![json!({"write": [name, "lyrics"]})], &["--worker", "go"]);
    let accepted = accept(&s, "fix-turkish", &[]);
    assert_eq!(accepted["state"], "accepted", "{accepted}");
    assert_eq!(accepted["files"], json!([name]), "{accepted}");
    assert_eq!(std::fs::read_to_string(repo.join(name)).unwrap(), "lyrics");
    git(&repo, &["cat-file", "-e", &format!("HEAD:{name}")]);
}

#[test]
fn accept_honours_the_repos_identity_and_hooks() {
    let s = Session::new();
    s.env("GIT_CONFIG_COUNT", "0");
    let repo = s.repo();
    run_one(&s, &repo, "fix-2", vec![json!({"write": ["made.txt", "hello"]})], &["--worker", "go"]);
    git(&repo, &["config", "user.name", "Repo Person"]);
    git(&repo, &["config", "user.email", "repo@example.test"]);
    one_hook(&repo, "pre-commit", "printf ran > .git/pre-commit-ran");
    let accepted = accept(&s, "fix-2", &["--message", "commit it"]);
    assert_eq!(accepted["state"], "accepted", "{accepted}");
    assert_eq!(std::fs::read_to_string(repo.join(".git").join("pre-commit-ran")).unwrap().trim(), "ran");
    assert_eq!(git(&repo, &["log", "-1", "--format=%an"]).trim(), "Repo Person");
    assert_eq!(git(&repo, &["log", "-1", "--format=%s"]).trim(), "commit it");
}

#[test]
fn accept_no_commit_leaves_the_changes_in_the_tree() {
    let s = Session::new();
    let repo = s.repo();
    run_one(&s, &repo, "fix-3", vec![json!({"write": ["made.txt", "hello"]})], &["--worker", "go"]);
    let worktree = worktree_of(&s, "fix-3");
    let head = git(&repo, &["rev-parse", "HEAD"]);
    let accepted = accept(&s, "fix-3", &["--no-commit"]);
    assert_eq!(accepted["state"], "accepted", "{accepted}");
    assert!(accepted["commit"].is_null(), "{accepted}");
    assert_eq!(std::fs::read_to_string(repo.join("made.txt")).unwrap(), "hello");
    assert_eq!(git(&repo, &["rev-parse", "HEAD"]), head, "nothing was committed");
    assert!(git(&repo, &["status", "--porcelain", "--", "made.txt"]).contains("made.txt"));
    assert!(!worktree.exists(), "the worktree is gone");
}

#[test]
fn accept_refuses_a_running_task() {
    let s = Session::new();
    let repo = s.repo();
    let script = s.script("accept-running", json!([[{"sleep_ms": 30000}]]));
    s.config(&script, "");
    s.start("fix-4", "sleep", &repo, &["--worker", "go"]);
    let error = refused(&s, &["task", "accept", "fix-4"]);
    assert_eq!(error["error"]["kind"], "running", "{error}");
    assert!(error["error"]["hint"].as_str().unwrap().contains("cancel"), "{error}");
    s.json(&["task", "cancel", "fix-4"]);
}

#[test]
fn accept_refuses_twice() {
    let s = Session::new();
    let repo = s.repo();
    run_one(&s, &repo, "fix-5", vec![json!({"write": ["made.txt", "hello"]})], &["--worker", "go"]);
    let accepted = accept(&s, "fix-5", &[]);
    assert_eq!(accepted["state"], "accepted", "{accepted}");
    let error = refused(&s, &["task", "accept", "fix-5"]);
    assert_eq!(error["error"]["kind"], "finished", "{error}");
}

#[test]
fn accept_refuses_a_failed_task_unless_told() {
    let s = Session::new();
    let repo = s.repo();
    let script = s.script("accept-failed", json!([[{"write": ["made.txt", "hello"]}, {"err": "boom"}, {"exit": 1}]]));
    s.config(&script, "");
    s.start("fix-6", "fail", &repo, &["--worker", "go"]);
    let (exit, rows, _) = s.wait(&["fix-6"], &["--timeout", "60"]);
    assert_eq!(exit, 1, "{rows:?}");
    assert_eq!(rows[0]["state"], "failed", "{rows:?}");
    let error = refused(&s, &["task", "accept", "fix-6"]);
    assert_eq!(error["error"]["kind"], "unfinished", "{error}");
    assert!(error["error"]["hint"].as_str().unwrap().contains("accept_unfinished"), "{error}");
    let accepted = accept(&s, "fix-6", &["--accept-unfinished"]);
    assert_eq!(accepted["state"], "accepted", "{accepted}");
    assert!(repo.join("made.txt").is_file(), "its work came in");
}

#[test]
fn accept_refuses_files_out_of_scope_unless_told() {
    let s = Session::new();
    let repo = s.repo();
    run_one(
        &s,
        &repo,
        "fix-7",
        vec![json!({"write": ["outside.txt", "mine"]})],
        &["--worker", "go", "--scope", "src/**"],
    );
    let error = refused(&s, &["task", "accept", "fix-7"]);
    assert_eq!(error["error"]["kind"], "scope_violation", "{error}");
    assert!(error["error"]["message"].as_str().unwrap().contains("outside.txt"), "{error}");
    let accepted = accept(&s, "fix-7", &["--accept-scope-violation"]);
    assert_eq!(accepted["state"], "accepted", "{accepted}");
    assert_eq!(accepted["files"], json!(["outside.txt"]), "{accepted}");
}

#[test]
fn accept_refuses_a_changed_branch() {
    let s = Session::new();
    let repo = s.repo();
    run_one(&s, &repo, "fix-8", vec![json!({"write": ["made.txt", "hello"]})], &["--worker", "go"]);
    let branch = branch_of(&s, "fix-8");
    git(&repo, &["checkout", "-q", "-b", "elsewhere"]);
    let error = refused(&s, &["task", "accept", "fix-8"]);
    assert_eq!(error["error"]["kind"], "target_branch_changed", "{error}");
    assert!(error["error"]["message"].as_str().unwrap().contains("elsewhere"), "{error}");
    git(&repo, &["checkout", "-q", &branch]);
    let accepted = accept(&s, "fix-8", &[]);
    assert_eq!(accepted["state"], "accepted", "{accepted}");
}

#[test]
fn accept_refuses_a_file_with_your_changes() {
    let s = Session::new();
    let repo = s.repo();
    run_one(&s, &repo, "fix-9", vec![json!({"write": ["a.txt", "one\nworker\n"]})], &["--worker", "go"]);
    std::fs::write(repo.join("a.txt"), "one\nmine\n").unwrap();
    let error = refused(&s, &["task", "accept", "fix-9"]);
    assert_eq!(error["error"]["kind"], "files_dirty", "{error}");
    assert!(error["error"]["message"].as_str().unwrap().contains("a.txt"), "{error}");
    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    let accepted = accept(&s, "fix-9", &[]);
    assert_eq!(accepted["state"], "accepted", "{accepted}");
    assert_eq!(std::fs::read_to_string(repo.join("a.txt")).unwrap().replace("\r\n", "\n"), "one\nworker\n");
}

#[test]
fn accept_refuses_no_changes() {
    let s = Session::new();
    let repo = s.repo();
    run_one(&s, &repo, "fix-10", vec![], &["--worker", "go"]);
    let error = refused(&s, &["task", "accept", "fix-10"]);
    assert_eq!(error["error"]["kind"], "no_changes", "{error}");
    assert!(error["error"]["hint"].as_str().unwrap().contains("discard"), "{error}");
    s.json(&["task", "discard", "fix-10"]);
}

#[test]
fn accept_refuses_a_conflict_and_leaves_the_repo_untouched() {
    let s = Session::new();
    let repo = s.repo();
    run_one(&s, &repo, "fix-11", vec![json!({"write": ["a.txt", "worker\n"]})], &["--worker", "go"]);
    std::fs::write(repo.join("a.txt"), "mine\n").unwrap();
    git(&repo, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-am", "mine"]);
    let head = git(&repo, &["rev-parse", "HEAD"]);
    let error = refused(&s, &["task", "accept", "fix-11"]);
    assert_eq!(error["error"]["kind"], "conflict", "{error}");
    assert_eq!(git(&repo, &["rev-parse", "HEAD"]), head, "no commit was made");
    assert_eq!(std::fs::read_to_string(repo.join("a.txt")).unwrap(), "mine\n", "the repository was not touched");
    assert!(git(&repo, &["status", "--porcelain"]).trim().is_empty(), "and it is still clean");
}

#[test]
fn a_refused_accept_keeps_its_review() {
    let s = Session::new();
    let repo = s.repo();
    run_one(
        &s,
        &repo,
        "fix-12",
        vec![json!({"write": ["outside.txt", "mine"]})],
        &["--worker", "go", "--scope", "src/**"],
    );
    let error = refused(&s, &["task", "accept", "fix-12", "--review", "looks good"]);
    assert_eq!(error["error"]["kind"], "scope_violation", "{error}");
    assert_eq!(s.task("fix-12")["task"]["review"], "looks good");
    s.json(&["task", "discard", "fix-12"]);
}

#[test]
fn a_failing_hook_says_so_and_leaves_the_changes() {
    let s = Session::new();
    s.env("GIT_CONFIG_COUNT", "0");
    let repo = s.repo();
    run_one(&s, &repo, "fix-13", vec![json!({"write": ["made.txt", "hello"]})], &["--worker", "go"]);
    one_hook(&repo, "pre-commit", "exit 1");
    let head = git(&repo, &["rev-parse", "HEAD"]);
    let error = refused(&s, &["task", "accept", "fix-13"]);
    assert_eq!(error["error"]["kind"], "commit_failed", "{error}");
    assert!(error["error"]["message"].as_str().unwrap().contains("working tree"), "{error}");
    assert_eq!(
        error["error"]["hint"],
        "the changes stay in the working tree: commit them yourself, then lc task discard fix-13",
        "{error}"
    );
    assert!(repo.join("made.txt").is_file(), "the changes are in the working tree");
    assert_eq!(git(&repo, &["rev-parse", "HEAD"]), head, "nothing was committed");
    assert!(git(&repo, &["status", "--porcelain", "--", "made.txt"]).contains("made.txt"));
}

#[test]
fn redo_resumes_on_the_same_worker_with_the_followup_alone() {
    let s = Session::new();
    let repo = s.repo();
    let kept = s.root.join("redo-prompt.md");
    let args = s.root.join("redo-args.txt");
    let mut first = vec![json!({"write": ["made.txt", "hello"]})];
    first.extend(say(&stream("ses_redo", "first done")));
    first.push(json!({"exit": 0}));
    let mut second =
        vec![json!({"args_to": args.display().to_string()}), json!({"file_arg_to": kept.display().to_string()})];
    second.extend(say(&stream("ses_redo", "second done")));
    second.push(json!({"exit": 0}));
    let script = s.script("redo-same", json!([first, second]));
    s.config(&script, "");
    s.start("fix-14", "do it", &repo, &["--worker", "go"]);
    let (exit, rows, _) = s.wait(&["fix-14"], &["--timeout", "60"]);
    assert_eq!(exit, 0, "{rows:?}");

    let brief = "now make it blue\n";
    let brief_file = s.brief("redo-same-brief", brief);
    let redone = s.json(&["task", "redo", "fix-14", "--brief", &brief_file]);
    assert_eq!(redone["turn"], 2, "{redone}");
    assert_eq!(redone["next"], "lc task wait fix-14", "{redone}");
    let (exit, rows, _) = s.wait(&["fix-14"], &["--timeout", "60"]);
    assert_eq!(exit, 0, "{rows:?}");
    assert_eq!(rows[0]["turns"], 2, "{rows:?}");

    let args = std::fs::read_to_string(&args).unwrap();
    let session_arg = args.lines().position(|line| line == "ses_redo").expect("the session is in argv");
    assert_eq!(args.lines().nth(session_arg.saturating_sub(1)), Some("--session"), "{args}");
    let prompt = std::fs::read_to_string(&kept).unwrap();
    assert_eq!(prompt, brief, "a resumed turn gets the follow-up alone");
}

#[test]
fn redo_on_a_blocked_worker_starts_fresh_with_a_note() {
    let s = Session::new();
    let repo = s.repo();
    let kept = s.root.join("blocked-prompt.md");
    s.json(&["workers", "--block", "go", "--until", "2099-01-01", "--reason", "out"]);
    let mut second = vec![json!({"file_arg_to": kept.display().to_string()})];
    second.extend(say(&stream("ses_go", "second done")));
    second.push(json!({"exit": 0}));
    let script = s.script("redo-blocked", json!([cc_turn("s_blocked", "the first report"), second]));
    s.config(&script, "");
    s.start("fix-15", "the original brief\n", &repo, &[]);
    let (exit, rows, _) = s.wait(&["fix-15"], &["--timeout", "60"]);
    assert_eq!(exit, 0, "{rows:?}");
    assert_eq!(rows[0]["worker"], "Command Code", "{rows:?}");

    s.json(&["workers", "--block", "cc", "--until", "2099-01-01", "--reason", "out"]);
    s.json(&["workers", "--clear", "go"]);
    let brief = "make it green\n";
    let brief_file = s.brief("redo-blocked-brief", brief);
    let redone = s.json(&["task", "redo", "fix-15", "--brief", &brief_file]);
    assert_eq!(redone["turn"], 2, "{redone}");
    let (exit, rows, _) = s.wait(&["fix-15"], &["--timeout", "60"]);
    assert_eq!(exit, 0, "{rows:?}");
    assert_eq!(rows[0]["worker"], "opencode Go", "{rows:?}");
    let prompt = std::fs::read_to_string(&kept).unwrap();
    assert!(prompt.contains("the original brief"), "{prompt}");
    assert!(prompt.contains(brief), "{prompt}");
    assert!(prompt.contains("the first report"), "the hand-off note carries the last report: {prompt}");
}

#[test]
fn redo_refuses_running_and_accepted() {
    let s = Session::new();
    let repo = s.repo();
    let script = s.script("redo-running", json!([[{"write": ["made.txt", "hello"]}, {"sleep_ms": 30000}]]));
    s.config(&script, "");
    s.start("fix-16", "long", &repo, &["--worker", "go"]);
    let worktree = worktree_of(&s, "fix-16");
    wait_until("the worker wrote its file", || worktree.join("made.txt").is_file());
    let brief = s.brief("redo-running-brief", "again\n");
    let error = refused(&s, &["task", "redo", "fix-16", "--brief", &brief]);
    assert_eq!(error["error"]["kind"], "running", "{error}");
    s.json(&["task", "cancel", "fix-16"]);
    let accepted = accept(&s, "fix-16", &["--accept-unfinished"]);
    assert_eq!(accepted["state"], "accepted", "{accepted}");
    let error = refused(&s, &["task", "redo", "fix-16", "--brief", &brief]);
    assert_eq!(error["error"]["kind"], "finished", "{error}");
}

#[test]
fn review_names_its_sections() {
    let s = Session::new();
    let repo = s.repo();
    run_one(&s, &repo, "fix-17", vec![json!({"write": ["made.txt", "hello"]})], &["--worker", "go"]);
    let review = review_fn(&s, "fix-17", &[]);
    let text = review["text"].as_str().unwrap();
    assert!(text.contains("## Checks"), "{text}");
    assert!(text.contains("No checks ran."), "{text}");
    assert!(text.contains("## Gaps\nNone."), "{text}");
    assert!(text.contains("## Report\nthe worker's report"), "{text}");
    assert!(text.contains("## Scope"), "{text}");
    assert!(text.contains("made.txt"), "{text}");
    assert_eq!(review["truncated"], false, "{review}");
    assert!(review["patch_file"].is_null(), "{review}");
    assert_eq!(review["state"], "done", "{review}");
}

#[test]
fn review_full_with_a_big_patch_goes_to_a_file() {
    let s = Session::new();
    let repo = s.repo();
    let big = "x".repeat(25_000);
    run_one(&s, &repo, "fix-18", vec![json!({"write": ["big.txt", big]})], &["--worker", "go"]);
    let review = review_fn(&s, "fix-18", &["--full"]);
    let file = review["patch_file"].as_str().unwrap_or_else(|| panic!("{review}"));
    let patch = std::fs::read_to_string(file).unwrap();
    assert!(patch.contains("big.txt"), "{}", &patch[..200.min(patch.len())]);
    assert!(patch.contains(&big), "the whole patch is in the file");
    assert!(review["text"].as_str().unwrap().contains(file), "the review names the file: {review}");
}

#[test]
fn review_after_accept_reads_the_kept_patch() {
    let s = Session::new();
    let repo = s.repo();
    run_one(&s, &repo, "fix-19", vec![json!({"write": ["made.txt", "hello"]})], &["--worker", "go"]);
    let accepted = accept(&s, "fix-19", &[]);
    assert_eq!(accepted["state"], "accepted", "{accepted}");
    let review = review_fn(&s, "fix-19", &["--full"]);
    assert_eq!(review["state"], "accepted", "{review}");
    assert!(review["patch_file"].is_null(), "{review}");
    let text = review["text"].as_str().unwrap();
    assert!(text.contains("made.txt"), "{text}");
    assert!(text.contains("+hello"), "the kept patch is shown: {text}");
}
