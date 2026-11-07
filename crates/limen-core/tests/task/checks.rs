//! The checks at a turn's end, and the QA rounds they start.

use super::*;

const QA_PROMPT: &str = "The acceptance checks failed after your turn. Fix what they name, inside the same \
                          scope, run the tests again, and report in the same shape as before.\n\n";

/// A shell line that runs the test worker with this script.
fn test_line(script: &Path) -> String {
    let quote = |path: &Path| {
        let text = path.display().to_string();
        if cfg!(windows) { format!("\"{text}\"") } else { format!("'{}'", text.replace('\'', "'\\''")) }
    };
    format!("{} {}", quote(Path::new(TEST_WORKER)), quote(script))
}

fn commit_checks(repo: &Path, checks: Value) {
    let dir = repo.join(".limen");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("checks.json"), serde_json::to_vec_pretty(&checks).unwrap()).unwrap();
    git(repo, &["-c", "user.name=t", "-c", "user.email=t@t", "add", "."]);
    git(repo, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "checks"]);
}

fn count(script: &Path) -> String {
    std::fs::read_to_string(format!("{}.count", script.display())).unwrap_or_default().trim().to_string()
}

/// One turn that writes `write` and then says a good report.
fn worked(session: &str, text: &str, write: Value) -> Vec<Value> {
    let mut actions = vec![write];
    actions.extend(say(&stream(session, text)));
    actions
}

fn named<'a>(list: &'a Value, name: &str) -> Option<&'a Value> {
    list["terminals"].as_array()?.iter().find(|terminal| terminal["name"] == name)
}

fn folder(s: &Session, name: &str) -> PathBuf {
    PathBuf::from(s.task(name)["folder"].as_str().unwrap())
}

#[cfg(windows)]
fn kill_pid(pid: u64) {
    let _ = std::process::Command::new("taskkill").args(["/PID", &pid.to_string(), "/F"]).output();
}

#[cfg(unix)]
fn kill_pid(pid: u64) {
    unsafe { libc::kill(pid as i32, libc::SIGKILL) };
}

#[test]
fn a_passing_test_finishes_the_task() {
    let s = Session::new();
    let repo = s.repo();
    let worker = s.script("worker-pass", json!([worked("ses_pass", "done", json!({"write": ["src/a.rs", "x\n"]}))]));
    s.config(&worker, "");
    let test = s.script("test-pass", json!([[{"out": "all good"}, {"exit": 0}]]));
    s.start("pass-1", "do it", &repo, &["--worker", "go", "--test", &test_line(&test)]);
    let (exit, rows, _) = s.wait(&["pass-1"], &["--timeout", "60"]);
    assert_eq!(exit, 0, "{rows:?}");
    assert_eq!(rows[0]["state"], "done", "{rows:?}");
    assert_eq!(rows[0]["ok"], true, "{rows:?}");
    assert_eq!(rows[0]["checks"], "passed", "{rows:?}");
    let view = s.task("pass-1");
    let report = &view["task"]["checks"][0];
    assert_eq!(report["ok"], true, "{view}");
    assert_eq!(report["test"]["ok"], true, "{view}");
    assert_eq!(report["test"]["exit_code"], 0, "{view}");
    assert!(report["test"]["log"].as_str().unwrap().ends_with("check-1.log"), "{view}");
}

#[test]
fn no_check_at_all_says_none_ran() {
    let s = Session::new();
    let repo = s.repo();
    let worker = s.script("worker-none", json!([worked("ses_none", "done", json!({"write": ["src/a.rs", "x\n"]}))]));
    s.config(&worker, "");
    s.start("none-1", "no checks", &repo, &["--worker", "go"]);
    let (exit, rows, _) = s.wait(&["none-1"], &["--timeout", "60"]);
    assert_eq!(exit, 0, "{rows:?}");
    assert_eq!(rows[0]["state"], "done", "{rows:?}");
    assert_eq!(rows[0]["ok"], true, "{rows:?}");
    assert_eq!(rows[0]["checks"], "none ran", "{rows:?}");
    assert!(s.task("none-1")["task"]["checks"][0]["test"].is_null());
}

#[test]
fn a_failing_test_goes_back_to_the_worker_in_its_session() {
    let s = Session::new();
    let repo = s.repo();
    let args = s.root.join("second-args.txt");
    let mut second = vec![json!({"args_to": args.display().to_string()})];
    second.push(json!({"write": ["src/a.rs", "fixed\n"]}));
    second.extend(say(&stream("ses_qa", "fixed it")));
    let worker = s.script(
        "worker-qa",
        json!([worked("ses_qa", "first try", json!({"write": ["src/a.rs", "broken\n"]})), second]),
    );
    s.config(&worker, "");
    let test = s.script(
        "test-qa",
        json!([
            [{"out": "FAILED tests/test_a.py::test_a"}, {"exit": 1}],
            [{"out": "OK"}, {"exit": 0}],
        ]),
    );
    s.start("qa-1", "fix it", &repo, &["--worker", "go", "--test", &test_line(&test)]);
    let (exit, rows, _) = s.wait(&["qa-1"], &["--timeout", "90"]);
    assert_eq!(exit, 0, "{rows:?}");
    assert_eq!(rows[0]["state"], "done", "{rows:?}");
    assert_eq!(rows[0]["ok"], true, "{rows:?}");
    assert_eq!(rows[0]["checks"], "passed", "{rows:?}");

    let view = s.task("qa-1");
    assert_eq!(view["task"]["qa_used"], 1, "{view}");
    let turns = view["task"]["turns"].as_array().unwrap();
    assert_eq!(turns.len(), 2, "{view}");
    assert_eq!(turns[1]["role"], "qa", "{view}");
    assert_eq!(turns[1]["resumes"], "ses_qa", "{view}");
    assert_eq!(turns[1]["session"], "ses_qa", "{view}");
    let written = std::fs::read_to_string(&args).unwrap();
    assert!(written.lines().any(|line| line == "--session"), "{written}");
    assert!(written.lines().any(|line| line == "ses_qa"), "{written}");

    let folder = folder(&s, "qa-1");
    let followup = std::fs::read_to_string(folder.join("turn-2.followup.md")).unwrap();
    assert!(followup.starts_with(QA_PROMPT), "{followup}");
    assert!(followup.contains("failed: tests/test_a.py::test_a"), "{followup}");
    let prompt = std::fs::read_to_string(folder.join("turn-2.prompt.md")).unwrap();
    assert_eq!(prompt, followup, "a resumed turn reads the follow-up alone");

    let reports = view["task"]["checks"].as_array().unwrap();
    assert_eq!(reports.len(), 2, "{view}");
    assert_eq!(reports[0]["ok"], false, "{view}");
    assert_eq!(reports[0]["test"]["ok"], false, "{view}");
    assert_eq!(reports[1]["test"]["ok"], true, "{view}");
}

#[test]
fn a_failing_test_with_no_rounds_ends_not_ok() {
    let s = Session::new();
    let repo = s.repo();
    let worker = s.script("worker-fail", json!([worked("ses_fail", "did it", json!({"write": ["src/a.rs", "x\n"]}))]));
    s.config(&worker, "");
    let test = s.script("test-fail", json!([[{"out": "FAILED tests/test_a.py::test_a"}, {"exit": 1}]]));
    s.start("fail-1", "fail", &repo, &["--worker", "go", "--test", &test_line(&test), "--qa-rounds", "0"]);
    let (exit, rows, _) = s.wait(&["fail-1"], &["--timeout", "60"]);
    assert_eq!(exit, 1, "{rows:?}");
    assert_eq!(rows[0]["state"], "done", "{rows:?}");
    assert_eq!(rows[0]["ok"], false, "{rows:?}");
    assert!(rows[0]["checks"].as_str().unwrap().starts_with("failed: "), "{rows:?}");
    let view = s.task("fail-1");
    assert_eq!(view["task"]["turns"].as_array().unwrap().len(), 1, "{view}");
    assert_eq!(view["task"]["checks"][0]["test"]["failed"][0], "tests/test_a.py::test_a", "{view}");
}

#[test]
fn rounds_run_out() {
    let s = Session::new();
    let repo = s.repo();
    let worker = s.script(
        "worker-rounds",
        json!([
            worked("ses_rounds", "one", json!({"write": ["src/a.rs", "1\n"]})),
            worked("ses_rounds", "two", json!({"write": ["src/a.rs", "2\n"]})),
            worked("ses_rounds", "three", json!({"write": ["src/a.rs", "3\n"]})),
        ]),
    );
    s.config(&worker, "");
    let test = s.script("test-rounds", json!([[{"out": "FAILED tests/test_a.py::test_a"}, {"exit": 1}]]));
    s.start("rounds-1", "rounds", &repo, &["--worker", "go", "--test", &test_line(&test), "--qa-rounds", "2"]);
    let (exit, rows, _) = s.wait(&["rounds-1"], &["--timeout", "120"]);
    assert_eq!(exit, 1, "{rows:?}");
    assert_eq!(rows[0]["state"], "done", "{rows:?}");
    assert_eq!(rows[0]["ok"], false, "{rows:?}");
    let view = s.task("rounds-1");
    assert_eq!(view["task"]["qa_used"], 2, "{view}");
    assert_eq!(view["task"]["turns"].as_array().unwrap().len(), 3, "{view}");
    assert_eq!(view["task"]["checks"].as_array().unwrap().len(), 3, "{view}");
}

#[test]
fn a_forbidden_line_fails_it() {
    let s = Session::new();
    let repo = s.repo();
    let worker =
        s.script("worker-forbid", json!([worked("ses_forbid", "done", json!({"write": ["src/bad.rs", "EVAL(x)\n"]}))]));
    s.config(&worker, "");
    s.start("forbid-1", "forbid", &repo, &["--worker", "go", "--forbid", "EVAL\\(", "--qa-rounds", "0"]);
    let (exit, rows, _) = s.wait(&["forbid-1"], &["--timeout", "60"]);
    assert_eq!(exit, 1, "{rows:?}");
    assert_eq!(rows[0]["ok"], false, "{rows:?}");
    assert!(rows[0]["checks"].as_str().unwrap().contains("forbidden line"), "{rows:?}");
    let view = s.task("forbid-1");
    assert_eq!(view["task"]["checks"][0]["forbid_checked"], 1, "{view}");
    assert_eq!(view["task"]["checks"][0]["forbid_hits"][0]["file"], "src/bad.rs", "{view}");
}

#[test]
fn a_missing_require_fails_it() {
    let s = Session::new();
    let repo = s.repo();
    let worker = s.script(
        "worker-require",
        json!([worked("ses_require", "done", json!({"write": ["src/a.rs", "pub fn a() {}\n"]}))]),
    );
    s.config(&worker, "");
    s.start("require-1", "require", &repo, &["--worker", "go", "--require", "src/**::fn gone", "--qa-rounds", "0"]);
    let (exit, rows, _) = s.wait(&["require-1"], &["--timeout", "60"]);
    assert_eq!(exit, 1, "{rows:?}");
    assert_eq!(rows[0]["ok"], false, "{rows:?}");
    assert!(rows[0]["checks"].as_str().unwrap().contains("required pattern missing"), "{rows:?}");
    let view = s.task("require-1");
    assert_eq!(view["task"]["checks"][0]["require_checked"], 1, "{view}");
    assert_eq!(view["task"]["checks"][0]["require_missing"][0], "src/**::fn gone", "{view}");
}

#[test]
fn a_changed_checks_file_fails_it() {
    let s = Session::new();
    let repo = s.repo();
    commit_checks(&repo, json!({}));
    let worker = s.script(
        "worker-checks-file",
        json!([worked("ses_checks_file", "done", json!({"write": [".limen/checks.json", "{\"qa_rounds\": 0}\n"]}))]),
    );
    s.config(&worker, "");
    s.start("checks-file-1", "change the checks", &repo, &["--worker", "go"]);
    let (exit, rows, _) = s.wait(&["checks-file-1"], &["--timeout", "60"]);
    assert_eq!(exit, 1, "{rows:?}");
    assert_eq!(rows[0]["ok"], false, "{rows:?}");
    assert!(rows[0]["checks"].as_str().unwrap().contains(".limen/checks.json changed"), "{rows:?}");
    let view = s.task("checks-file-1");
    assert_eq!(view["task"]["checks"][0]["checks_file_changed"], true, "{view}");
    assert_eq!(view["task"]["turns"].as_array().unwrap().len(), 2, "the change goes back for a round: {view}");
    assert_eq!(view["task"]["qa_used"], 1, "{view}");
}

#[test]
fn a_warn_flags_and_never_fails() {
    let s = Session::new();
    let repo = s.repo();
    let worker = s
        .script("worker-warn", json!([worked("ses_warn", "done", json!({"write": ["src/w.rs", "panic!(\"x\");\n"]}))]));
    s.config(&worker, "");
    s.start("warn-1", "warn", &repo, &["--worker", "go", "--warn", "src/**::panic!", "--qa-rounds", "0"]);
    let (exit, rows, _) = s.wait(&["warn-1"], &["--timeout", "60"]);
    assert_eq!(exit, 0, "{rows:?}");
    assert_eq!(rows[0]["state"], "done", "{rows:?}");
    assert_eq!(rows[0]["ok"], true, "a warn never fails a task: {rows:?}");
    let view = s.task("warn-1");
    let hits = view["task"]["checks"][0]["warn_hits"].as_array().unwrap();
    assert_eq!(hits.len(), 1, "{view}");
    assert_eq!(hits[0]["rule"], "src/**::panic!", "{view}");
    assert_eq!(hits[0]["source"], "call", "{view}");
}

#[test]
fn a_test_past_its_time_is_killed() {
    let s = Session::new();
    let repo = s.repo();
    let worker =
        s.script("worker-timeout", json!([worked("ses_timeout", "done", json!({"write": ["src/a.rs", "x\n"]}))]));
    s.config(&worker, "");
    let test = s.script(
        "test-timeout",
        json!([[{"out": "FAILED tests/test_slow.py::test_slow"}, {"out": "still running"}, {"sleep_ms": 60000}]]),
    );
    commit_checks(&repo, json!({"test_cmd": test_line(&test), "test_timeout_seconds": 1}));
    s.start("timeout-1", "timeout", &repo, &["--worker", "go", "--qa-rounds", "0"]);
    let (exit, rows, _) = s.wait(&["timeout-1"], &["--timeout", "90"]);
    assert_eq!(exit, 1, "{rows:?}");
    assert_eq!(rows[0]["ok"], false, "{rows:?}");
    assert!(rows[0]["checks"].as_str().unwrap().contains("the test timed out"), "{rows:?}");
    let view = s.task("timeout-1");
    let report = &view["task"]["checks"][0];
    assert_eq!(report["test"]["timed_out"], true, "{view}");
    assert!(report["test"]["exit_code"].is_null(), "{view}");
    assert_eq!(report["test"]["failed"][0], "tests/test_slow.py::test_slow", "it kept what it printed: {view}");
    assert!(report["test"]["tail"].as_str().unwrap().contains("FAILED tests/test_slow.py::test_slow"), "{view}");
    assert!(report["test"]["log"].as_str().is_some(), "{view}");
    wait_until("the test's terminal ended", || {
        named(&s.json(&["ls", "--all"]), "timeout-1.check").is_some_and(|terminal| terminal["state"] == "finished")
    });
}

#[test]
fn cancel_during_a_check_ends_it() {
    let s = Session::new();
    let repo = s.repo();
    let worker =
        s.script("worker-cancel", json!([worked("ses_cancel", "done", json!({"write": ["src/a.rs", "x\n"]}))]));
    s.config(&worker, "");
    let test = s.script("test-cancel", json!([[{"sleep_ms": 60000}]]));
    s.start("cancel-1", "cancel a check", &repo, &["--worker", "go", "--test", &test_line(&test)]);
    wait_until("the test started", || !s.task("cancel-1")["task"]["check"].is_null());
    let cancelled = s.json(&["task", "cancel", "cancel-1"]);
    assert_eq!(cancelled["state"], "cancelled", "{cancelled}");
    let view = s.task("cancel-1");
    assert!(view["task"]["check"].is_null(), "{view}");
    assert!(view["task"]["ended_ms"].as_u64().is_some(), "{view}");
    wait_until("the test's terminal ended", || {
        named(&s.json(&["ls", "--all"]), "cancel-1.check").is_some_and(|terminal| terminal["state"] == "finished")
    });
}

#[test]
fn a_round_that_changed_only_markdown_reuses_the_test() {
    let s = Session::new();
    let repo = s.repo();
    let mut first =
        vec![json!({"write": ["notes.md", "EVIL line\n"]}), json!({"write": ["src/keep.rs", "pub fn keep() {}\n"]})];
    first.extend(say(&stream("ses_md", "wrote a note")));
    let mut second = vec![json!({"write": ["notes.md", "clean now\n"]})];
    second.extend(say(&stream("ses_md", "fixed the note")));
    let worker = s.script("worker-md", json!([first, second]));
    s.config(&worker, "");
    let test = s.script("test-md", json!([[{"out": "OK"}, {"exit": 0}]]));
    s.start("md-1", "only markdown moves", &repo, &["--worker", "go", "--test", &test_line(&test), "--forbid", "EVIL"]);
    let (exit, rows, _) = s.wait(&["md-1"], &["--timeout", "90"]);
    assert_eq!(exit, 0, "{rows:?}");
    assert_eq!(rows[0]["ok"], true, "{rows:?}");
    assert_eq!(count(&test), "1", "the test ran once");
    let view = s.task("md-1");
    let reports = view["task"]["checks"].as_array().unwrap();
    assert_eq!(reports.len(), 2, "{view}");
    assert_eq!(reports[0]["ok"], false, "{view}");
    let reused = reports[1]["test"]["reused"].as_str().unwrap();
    assert_eq!(reused, "the round changed only files matching reuse_test_for (notes.md)", "{view}");
    assert!(reports[1]["test"]["ok"].as_bool().unwrap(), "{view}");
}

#[test]
fn a_round_that_changed_code_runs_it_again() {
    let s = Session::new();
    let repo = s.repo();
    let mut first =
        vec![json!({"write": ["notes.md", "EVIL line\n"]}), json!({"write": ["src/keep.rs", "pub fn keep() {}\n"]})];
    first.extend(say(&stream("ses_code", "wrote code")));
    let mut second =
        vec![json!({"write": ["notes.md", "clean now\n"]}), json!({"write": ["src/keep.rs", "pub fn changed() {}\n"]})];
    second.extend(say(&stream("ses_code", "fixed both")));
    let worker = s.script("worker-code", json!([first, second]));
    s.config(&worker, "");
    let test = s.script("test-code", json!([[{"out": "OK"}, {"exit": 0}]]));
    s.start("code-1", "code moves too", &repo, &["--worker", "go", "--test", &test_line(&test), "--forbid", "EVIL"]);
    let (exit, rows, _) = s.wait(&["code-1"], &["--timeout", "90"]);
    assert_eq!(exit, 0, "{rows:?}");
    assert_eq!(rows[0]["ok"], true, "{rows:?}");
    assert_eq!(count(&test), "2", "the test ran again");
    let view = s.task("code-1");
    let reports = view["task"]["checks"].as_array().unwrap();
    assert_eq!(reports.len(), 2, "{view}");
    assert!(reports[1]["test"]["reused"].is_null(), "{view}");
}

#[test]
fn a_check_survives_a_core_restart() {
    let s = Session::new();
    let repo = s.repo();
    let worker = s.script(
        "worker-restart",
        json!([worked("ses_check_restart", "done", json!({"write": ["src/a.rs", "fn done() {}\n"]}))]),
    );
    s.config(&worker, "");
    let test = s.script("test-restart", json!([[{"sleep_ms": 6000}, {"out": "OK"}, {"exit": 0}]]));
    s.start(
        "check-restart-1",
        "survive",
        &repo,
        &["--worker", "go", "--test", &test_line(&test), "--require", "src/a.rs::fn done"],
    );
    wait_until("the test started", || !s.task("check-restart-1")["task"]["check"].is_null());
    let (code, _, err) = s.out(&["server", "restart"]);
    assert_eq!(code, 0, "{err}");
    let (exit, rows, _) = s.wait(&["check-restart-1"], &["--timeout", "90"]);
    assert_eq!(exit, 0, "{rows:?}");
    assert_eq!(rows[0]["state"], "done", "{rows:?}");
    assert_eq!(rows[0]["ok"], true, "{rows:?}");
    let view = s.task("check-restart-1");
    let report = &view["task"]["checks"][0];
    assert_eq!(report["test"]["ok"], true, "{view}");
    assert!(report["errors"].as_array().unwrap().is_empty(), "{view}");
    assert_eq!(report["require_checked"], 1, "the patterns' half survived too: {view}");
    assert!(report["require_missing"].as_array().unwrap().is_empty(), "{view}");
}

#[test]
fn errors_alone_are_not_sent_back() {
    let s = Session::new();
    let repo = s.repo();
    let worker =
        s.script("worker-errors", json!([worked("ses_errors", "done", json!({"write": ["src/a.rs", "x\n"]}))]));
    s.config(&worker, "");
    s.start("errors-1", "bad pattern", &repo, &["--worker", "go", "--forbid", "(["]);
    let (exit, rows, _) = s.wait(&["errors-1"], &["--timeout", "60"]);
    assert_eq!(exit, 1, "{rows:?}");
    assert_eq!(rows[0]["state"], "done", "the checks ended it: {rows:?}");
    assert_eq!(rows[0]["ok"], false, "{rows:?}");
    let view = s.task("errors-1");
    assert_eq!(view["task"]["turns"].as_array().unwrap().len(), 1, "a bad pattern is not sent back: {view}");
    assert!(!view["task"]["checks"][0]["errors"].as_array().unwrap().is_empty(), "{view}");
}

#[test]
fn a_check_shows_as_its_own_terminal() {
    let s = Session::new();
    let repo = s.repo();
    let worker = s.script("worker-show", json!([worked("ses_show", "done", json!({"write": ["src/a.rs", "x\n"]}))]));
    s.config(&worker, "");
    let test = s.script("test-show", json!([[{"out": "the check is running"}, {"sleep_ms": 4000}, {"exit": 0}]]));
    s.start("show-1", "watch the check", &repo, &["--worker", "go", "--test", &test_line(&test)]);
    wait_until("the check's terminal", || named(&s.json(&["ls"]), "show-1.check").is_some());
    let terminal = s.json(&["get", "show-1.check"]);
    assert_eq!(terminal["labels"]["task"], "show-1", "{terminal}");
    assert_eq!(terminal["labels"]["check"], "check", "{terminal}");
    assert_eq!(terminal["contain"], true, "{terminal}");
    wait_until("the check's screen", || s.read("show-1.check").contains("the check is running"));
    let (exit, rows, _) = s.wait(&["show-1"], &["--timeout", "60"]);
    assert_eq!(exit, 0, "{rows:?}");
    assert_eq!(rows[0]["state"], "done", "{rows:?}");
}

#[test]
fn a_child_holding_the_pipe_does_not_hide_a_passing_test() {
    let s = Session::new();
    let repo = s.repo();
    let worker =
        s.script("worker-orphan", json!([worked("ses_orphan", "done", json!({"write": ["src/a.rs", "x\n"]}))]));
    s.config(&worker, "");
    let pidfile = s.root.join("orphan.pid");
    let test =
        s.script("test-orphan", json!([[{"orphan": pidfile.display().to_string()}, {"out": "OK"}, {"exit": 0}]]));
    s.start("orphan-1", "hold the pipe", &repo, &["--worker", "go", "--test", &test_line(&test), "--qa-rounds", "0"]);
    let (exit, rows, _) = s.wait(&["orphan-1"], &["--timeout", "120"]);
    assert_eq!(exit, 0, "{rows:?}");
    assert_eq!(rows[0]["state"], "done", "{rows:?}");
    assert_eq!(rows[0]["ok"], true, "{rows:?}");
    assert_eq!(rows[0]["checks"], "passed", "{rows:?}");
    let view = s.task("orphan-1");
    assert_eq!(view["task"]["checks"][0]["test"]["exit_code"], 0, "{view}");
    if let Ok(text) = std::fs::read_to_string(&pidfile)
        && let Ok(pid) = text.trim().parse::<u64>()
    {
        kill_pid(pid);
    }
}

#[test]
fn a_taken_check_name_is_started_under_another() {
    let s = Session::new();
    let repo = s.repo();
    let blocker = s.script("blocker", json!([[{"sleep_ms": 60000}]]));
    s.json(&["run", "--name", "x.check", "--bg", TEST_WORKER, &blocker.display().to_string()]);
    wait_until("the blocker holds x.check", || named(&s.json(&["ls"]), "x.check").is_some());
    let worker = s.script("worker-taken", json!([worked("ses_taken", "done", json!({"write": ["src/a.rs", "x\n"]}))]));
    s.config(&worker, "");
    let test = s.script("test-taken", json!([[{"out": "OK"}, {"exit": 0}]]));
    s.start("x", "name taken", &repo, &["--worker", "go", "--test", &test_line(&test)]);
    let (exit, rows, _) = s.wait(&["x"], &["--timeout", "60"]);
    assert_eq!(exit, 0, "{rows:?}");
    assert_eq!(rows[0]["ok"], true, "{rows:?}");
    assert_eq!(count(&test), "1", "the test still ran");
    let all = s.json(&["ls", "--all"]);
    assert!(named(&all, "x.1.check").is_some(), "the check took the fallback name: {all}");
    s.json(&["kill", "x.check"]);
}

#[test]
fn a_new_checks_file_is_not_a_change() {
    let s = Session::new();
    let repo = s.repo();
    let worker = s.script(
        "worker-new-checks",
        json!([worked(
            "ses_new_checks",
            "done",
            json!({"write": [".limen/checks.json", "{\"test_cmd\": \"exit 1\"}\n"]})
        )]),
    );
    s.config(&worker, "");
    s.start("new-checks-1", "add the checks file", &repo, &["--worker", "go"]);
    let (exit, rows, _) = s.wait(&["new-checks-1"], &["--timeout", "60"]);
    assert_eq!(exit, 0, "{rows:?}");
    assert_eq!(rows[0]["ok"], true, "a file added where none applied is no change: {rows:?}");
    assert_eq!(rows[0]["checks"], "none ran", "{rows:?}");
    assert_eq!(s.task("new-checks-1")["task"]["checks"][0]["checks_file_changed"], false);
}

#[test]
fn a_bad_checks_key_only_warns() {
    let s = Session::new();
    let repo = s.repo();
    let test = s.script("test-bad-key", json!([[{"out": "OK"}, {"exit": 0}]]));
    commit_checks(&repo, json!({"test_cmd": test_line(&test), "qa_rounds": "soon"}));
    let worker =
        s.script("worker-bad-key", json!([worked("ses_bad_key", "done", json!({"write": ["src/a.rs", "x\n"]}))]));
    s.config(&worker, "");
    s.start("bad-key-1", "bad key", &repo, &["--worker", "go"]);
    let (exit, rows, _) = s.wait(&["bad-key-1"], &["--timeout", "60"]);
    assert_eq!(exit, 0, "{rows:?}");
    assert_eq!(rows[0]["ok"], true, "a bad qa_rounds never fails a check: {rows:?}");
    let warnings = s.task("bad-key-1")["task"]["warnings"].as_array().unwrap().clone();
    let word = ".limen/checks.json@HEAD: qa_rounds must be an integer from 0 to 3";
    assert_eq!(warnings.iter().filter(|warning| warning.as_str() == Some(word)).count(), 1, "{warnings:?}");
}
