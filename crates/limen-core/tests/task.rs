//! The worker loop end to end: a real core, a real git repo, the real runner,
//! and the test worker in place of an OpenCode CLI. The same tests run on
//! Windows here and on Linux in CI.

use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_limen-core");
const TEST_WORKER: &str = env!("CARGO_BIN_EXE_limen-test-worker");
static LIVE: Mutex<Vec<(String, PathBuf)>> = Mutex::new(Vec::new());

struct Session {
    name: String,
    root: PathBuf,
}

impl Session {
    fn new() -> Session {
        static N: AtomicU32 = AtomicU32::new(0);
        let name = format!("task-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed));
        let root = std::env::temp_dir().join(format!("lc-{name}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        LIVE.lock().unwrap().push((name.clone(), root.clone()));
        Session { name, root }
    }

    fn cmd(&self) -> Command {
        let mut cmd = Command::new(BIN);
        cmd.current_dir(&self.root)
            .env("LIMEN_SESSION", &self.name)
            .env("XDG_STATE_HOME", self.root.join("state"))
            .env("XDG_CONFIG_HOME", self.root.join("config"))
            .env("LIMEN_SETTLE_MS", "0")
            .env_remove("LIMEN_TERMINAL_ID")
            .env_remove("LIMEN_SCOPE")
            .env_remove("LIMEN_SOCKET")
            .env_remove("LIMEN_WORKER")
            .env_remove("LIMEN_COMSPEC")
            .env_remove("SSH_CONNECTION")
            .env_remove("SSH_CLIENT")
            .env_remove("SSH_TTY");
        #[cfg(windows)]
        cmd.env("COMSPEC", real_cmd());
        no_window(&mut cmd);
        cmd
    }

    fn output(&self, args: &[&str]) -> Output {
        self.cmd().args(args).output().unwrap()
    }

    fn out(&self, args: &[&str]) -> (i32, String, String) {
        let out = self.output(args);
        if !out.stderr.is_empty() {
            eprintln!("{args:?}: {}", String::from_utf8_lossy(&out.stderr));
        }
        (
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    fn json(&self, args: &[&str]) -> Value {
        let (_, text, _) = self.out(args);
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("{args:?}: {e}: {text}"))
    }

    fn state(&self) -> PathBuf {
        self.root.join("state").join("limen-core").join(&self.name)
    }

    /// The config the core reads: one worker CLI, the test worker behind a script.
    fn config(&self, script: &Path, extra: &str) {
        let dir = self.root.join("config").join("limen-core");
        std::fs::create_dir_all(&dir).unwrap();
        let worker = serde_json::to_string(TEST_WORKER).unwrap();
        let script = serde_json::to_string(script.to_str().unwrap()).unwrap();
        let text = format!("[workers]\n{extra}opencode = [{worker}, {script}]\n");
        std::fs::write(dir.join("config.toml"), text).unwrap();
    }

    fn script(&self, name: &str, turns: Value) -> PathBuf {
        let path = self.root.join(format!("{name}.json"));
        std::fs::write(&path, serde_json::to_vec(&json!({"turns": turns})).unwrap()).unwrap();
        path
    }

    fn repo(&self) -> PathBuf {
        let dir = self.root.join("repo");
        std::fs::create_dir_all(&dir).unwrap();
        git(&dir, &["init", "-q"]);
        std::fs::write(dir.join("a.txt"), "one\n").unwrap();
        git(&dir, &["-c", "user.name=t", "-c", "user.email=t@t", "add", "."]);
        git(&dir, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "first"]);
        dir
    }

    fn brief(&self, name: &str, text: &str) -> String {
        let path = self.root.join(format!("{name}.brief.md"));
        std::fs::write(&path, text).unwrap();
        path.display().to_string()
    }

    fn start(&self, name: &str, brief: &str, repo: &Path, extra: &[&str]) -> Value {
        let brief = self.brief(name, brief);
        let folder = repo.display().to_string();
        let mut args: Vec<String> =
            vec!["task".into(), "start".into(), name.into(), "--brief".into(), brief, "--in".into(), folder];
        args.extend(extra.iter().map(|arg| arg.to_string()));
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        self.json(&args)
    }

    /// `lc task wait`, with the row for each task and the summary line.
    fn wait(&self, names: &[&str], extra: &[&str]) -> (i32, Vec<Value>, Value) {
        let mut args = vec!["task", "wait"];
        args.extend_from_slice(names);
        args.extend_from_slice(extra);
        let (code, text, _) = self.out(&args);
        let mut rows: Vec<Value> = text
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| serde_json::from_str(line).unwrap_or_else(|e| panic!("{e}: {line}")))
            .collect();
        let summary = rows.pop().expect("a summary line");
        (code, rows, summary)
    }

    fn task(&self, name: &str) -> Value {
        self.json(&["task", "show", name])
    }

    fn read(&self, name: &str) -> String {
        self.out(&["read", name]).1
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.cmd().args(["server", "stop"]).output();
        LIVE.lock().unwrap().retain(|(name, _)| *name != self.name);
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[cfg(windows)]
fn no_window(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
}

#[cfg(not(windows))]
fn no_window(_cmd: &mut Command) {}

#[cfg(windows)]
fn real_cmd() -> String {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    Path::new(&root).join(r"System32\cmd.exe").display().to_string()
}

fn git(dir: &Path, args: &[&str]) -> String {
    let mut cmd = Command::new("git");
    cmd.current_dir(dir).args(args).env("GIT_TERMINAL_PROMPT", "0");
    no_window(&mut cmd);
    let out = cmd.output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn wait_until(what: &str, mut ok: impl FnMut() -> bool) {
    let start = Instant::now();
    while !ok() {
        assert!(start.elapsed() < Duration::from_secs(30), "timed out waiting: {what}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(windows)]
fn alive(pid: u64) -> bool {
    let out = Command::new("tasklist").args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"]).output().unwrap();
    String::from_utf8_lossy(&out.stdout).contains(&format!("\"{pid}\""))
}

#[cfg(unix)]
fn alive(pid: u64) -> bool {
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

/// One well-formed OpenCode line per step.
fn stream(session: &str, text: &str) -> Vec<String> {
    vec![
        json!({"type":"step_start","sessionID":session,"part":{"type":"step-start"}}).to_string(),
        json!({"type":"text","sessionID":session,"part":{"type":"text","text":text}}).to_string(),
        json!({"type":"tool_use","sessionID":session,"part":{"partID":"prt_1","type":"tool","tool":"write","state":{"status":"completed","input":{"filePath":"b.txt"}}}}).to_string(),
        json!({"type":"step_finish","sessionID":session,"part":{"type":"step-finish","cost":0.25,"tokens":{"input":100,"output":20,"reasoning":0,"cache":{"read":5,"write":2}}}}).to_string(),
    ]
}

/// The lines as `out` actions, in order.
fn say(lines: &[String]) -> Vec<Value> {
    lines.iter().map(|line| json!({"out": line})).collect()
}

#[test]
fn a_task_runs_in_a_worktree_and_reports() {
    let s = Session::new();
    let repo = s.repo();
    let session = "ses_worktree";
    let report = "r".repeat(2000);
    let mut actions = vec![json!({"write": ["made.txt", "hello"]})];
    actions.extend(say(&stream(session, &report)));
    let script = s.script("one", json!([actions]));
    s.config(&script, "");
    let started = s.start("fix-1", "write made.txt", &repo, &[]);
    assert_eq!(started["state"], "running", "{started}");
    let (exit, rows, summary) = s.wait(&["fix-1"], &["--timeout", "30"]);
    assert_eq!(exit, 0, "{rows:?} {summary}");
    let line = &rows[0];
    assert_eq!(line["state"], "done", "{line}");
    assert_eq!(line["ok"], true, "{line}");
    assert_eq!(line["worker"], "opencode Go", "{line}");
    assert!((line["cost_usd"].as_f64().unwrap() - 0.25).abs() < 1e-9, "{line}");
    assert_eq!(line["text"].as_str().unwrap().chars().count(), 1500, "the wait cuts the report");
    assert_eq!(summary["summary"]["ok"], 1, "{summary}");

    let view = s.task("fix-1");
    let worktree = PathBuf::from(view["task"]["worktree"].as_str().unwrap());
    assert!(worktree.join("made.txt").is_file(), "the worker wrote into its worktree");
    assert!(!repo.join("made.txt").exists(), "and not into the repository");
    let result_path = Path::new(view["folder"].as_str().unwrap()).join("result.json");
    let result: Value = serde_json::from_slice(&std::fs::read(result_path).unwrap()).unwrap();
    assert_eq!(result["text"].as_str().unwrap(), report, "result.json holds it whole");

    let screen = s.read("fix-1");
    assert!(screen.contains("write b.txt"), "the runner's screen has the tool's line: {screen}");
    let terminal = s.json(&["get", "fix-1"]);
    assert_eq!(terminal["contain"], true);
    assert_eq!(terminal["labels"]["task"], "fix-1");
    assert_eq!(terminal["labels"]["worker"], "go");
}

#[test]
fn a_failed_worker_wakes_a_wait_for_several_at_once() {
    let s = Session::new();
    let repo = s.repo();
    // The turn counter is shared: the first worker sleeps, the second exits 3.
    let slow = json!([{"sleep_ms": 30000}]);
    let bad = json!([{"out": stream("ses_bad", "never")[0]}, {"exit": 3}]);
    let script = s.script("two", json!([slow, bad]));
    s.config(&script, "");
    s.start("slow", "sleep", &repo, &["--scope", "src/**"]);
    wait_until("the first worker took its turn", || {
        std::fs::read_to_string(format!("{}.count", script.display())).is_ok_and(|c| c.trim() == "1")
    });
    s.start("bad", "exit 3", &repo, &["--scope", "docs/**"]);
    let start = Instant::now();
    let (exit, rows, summary) = s.wait(&["bad", "slow"], &["--timeout", "20"]);
    assert!(start.elapsed() < Duration::from_secs(10), "a bad end wakes at once");
    assert_eq!(exit, 1, "{rows:?} {summary}");
    let bad_row = rows.iter().find(|row| row["task"] == "bad").unwrap();
    assert_eq!(bad_row["state"], "failed", "{bad_row}");
    assert!(bad_row["error"].as_str().unwrap().contains("code 3"), "{bad_row}");
    let slow_row = rows.iter().find(|row| row["task"] == "slow").unwrap();
    assert!(
        !matches!(slow_row["state"].as_str().unwrap(), "done" | "failed" | "timed_out" | "cancelled"),
        "{slow_row}"
    );
    assert_eq!(summary["summary"]["unfinished"], json!(["slow"]), "{summary}");
    let cancelled = s.json(&["task", "cancel", "slow"]);
    assert_eq!(cancelled["state"], "cancelled", "{cancelled}");
    assert_eq!(cancelled["survivors"], 0, "{cancelled}");
}

#[test]
fn an_error_event_fails_a_turn_that_exits_0() {
    let s = Session::new();
    let repo = s.repo();
    let error = json!({"type":"error","sessionID":"ses_error","error":{"name":"ProviderError","data":{"code":"x","message":"boom"}}});
    let script =
        s.script("error", json!([[{"out": error.to_string()}, {"out": stream("ses_error", "text")[3]}, {"exit": 0}]]));
    s.config(&script, "");
    s.start("fix-3", "error", &repo, &[]);
    let (exit, rows, _) = s.wait(&["fix-3"], &["--timeout", "30"]);
    assert_eq!(exit, 1, "{rows:?}");
    assert_eq!(rows[0]["state"], "failed", "{rows:?}");
    assert!(rows[0]["error"].as_str().unwrap().contains("ProviderError"), "{rows:?}");
}

#[test]
fn a_silent_worker_is_cut_at_its_idle_limit() {
    let s = Session::new();
    let repo = s.repo();
    let script = s.script("silent", json!([[{"sleep_ms": 60000}]]));
    s.config(&script, "");
    s.start("fix-4", "silent", &repo, &["--idle", "2"]);
    let (exit, rows, _) = s.wait(&["fix-4"], &["--timeout", "60"]);
    assert_eq!(exit, 1, "{rows:?}");
    assert_eq!(rows[0]["state"], "timed_out", "{rows:?}");
    assert!(rows[0]["error"].as_str().unwrap().contains("idle limit 2"), "{rows:?}");
}

#[test]
fn a_quiet_worker_using_cpu_is_not_idle() {
    let s = Session::new();
    let repo = s.repo();
    let mut actions = vec![json!({"burn_ms": 5000})];
    actions.extend(say(&stream("ses_burn", "all done")));
    let script = s.script("burn", json!([actions]));
    s.config(&script, "");
    s.start("fix-5", "burn", &repo, &["--idle", "2"]);
    let (exit, rows, _) = s.wait(&["fix-5"], &["--timeout", "60"]);
    assert_eq!(exit, 0, "{rows:?}");
    assert_eq!(rows[0]["state"], "done", "{rows:?}");
}

#[test]
fn a_worker_past_its_time_is_cut() {
    let s = Session::new();
    let repo = s.repo();
    let script = s.script("wall", json!([[{"sleep_ms": 60000}]]));
    s.config(&script, "");
    s.start("fix-6", "sleep past its time", &repo, &["--timeout", "2"]);
    let (exit, rows, _) = s.wait(&["fix-6"], &["--timeout", "60"]);
    assert_eq!(exit, 1, "{rows:?}");
    assert_eq!(rows[0]["state"], "timed_out", "{rows:?}");
    assert!(rows[0]["error"].as_str().unwrap().contains("2 s limit"), "{rows:?}");
}

#[test]
fn cancel_ends_the_worker_and_what_it_started() {
    let s = Session::new();
    let repo = s.repo();
    let pidfile = s.root.join("grandchild.pid");
    let pidfile = pidfile.display().to_string();
    let script = s.script("child", json!([[{"child": pidfile}, {"sleep_ms": 60000}]]));
    s.config(&script, "");
    s.start("fix-7", "start a child", &repo, &[]);
    let pid_path = s.root.join("grandchild.pid");
    wait_until("the grandchild's pid", || std::fs::read_to_string(&pid_path).is_ok_and(|text| !text.trim().is_empty()));
    let pid: u64 = std::fs::read_to_string(&pid_path).unwrap().trim().parse().unwrap();
    let cancelled = s.json(&["task", "cancel", "fix-7"]);
    assert_eq!(cancelled["state"], "cancelled", "{cancelled}");
    assert_eq!(cancelled["survivors"], 0, "{cancelled}");
    wait_until("the grandchild gone", || !alive(pid));
}

#[test]
fn a_task_survives_a_core_restart() {
    let s = Session::new();
    let repo = s.repo();
    let mut actions = vec![json!({"sleep_ms": 6000})];
    actions.extend(say(&stream("ses_restart", "still here")));
    let script = s.script("restart", json!([actions]));
    s.config(&script, "");
    s.start("fix-8", "survive a restart", &repo, &[]);
    std::thread::sleep(Duration::from_millis(1500));
    let (code, _, err) = s.out(&["server", "restart"]);
    assert_eq!(code, 0, "{err}");
    let (exit, rows, _) = s.wait(&["fix-8"], &["--timeout", "60"]);
    assert_eq!(exit, 0, "{rows:?}");
    assert_eq!(rows[0]["state"], "done", "{rows:?}");
    assert_eq!(rows[0]["turns"], 1, "{rows:?}");
}

#[test]
fn the_cap_and_overlapping_scopes_queue_in_order() {
    let s = Session::new();
    let repo = s.repo();
    let mut long = vec![json!({"sleep_ms": 4000})];
    long.extend(say(&stream("ses_a", "a done")));
    let mut short = vec![json!({"sleep_ms": 1000})];
    short.extend(say(&stream("ses_b", "b done")));
    let script = s.script("queue", json!([long, short, short]));
    s.config(&script, "cap = 2\n");
    s.start("a", "long", &repo, &["--scope", "src/**"]);
    let b = s.start("b", "overlapping", &repo, &["--scope", "src/x/**"]);
    assert_eq!(b["state"], "queued", "{b}");
    assert!(b["waiting"].as_str().unwrap().contains("a"), "{b}");
    let c = s.start("c", "disjoint", &repo, &["--scope", "docs/**"]);
    assert_eq!(c["state"], "running", "{c}");
    let (exit, rows, _) = s.wait(&["a"], &["--timeout", "30"]);
    assert_eq!(exit, 0, "{rows:?}");
    wait_until("the queued task started on its own", || {
        let task = s.task("b");
        matches!(task["task"]["state"].as_str().unwrap(), "running" | "done")
    });
    let (exit, rows, _) = s.wait(&["b", "c"], &["--timeout", "30"]);
    assert_eq!(exit, 0, "{rows:?}");
}

#[test]
fn a_change_outside_the_scope_is_marked() {
    let s = Session::new();
    let repo = s.repo();
    let mut actions = vec![json!({"write": ["outside.txt", "x"]})];
    actions.extend(say(&stream("ses_scope", "done")));
    let script = s.script("scope", json!([actions]));
    s.config(&script, "");
    s.start("fix-10", "leave the scope", &repo, &["--scope", "src/**"]);
    let (exit, rows, _) = s.wait(&["fix-10"], &["--timeout", "30"]);
    assert_eq!(exit, 1, "a change outside the scope is not ok: {rows:?}");
    assert_eq!(rows[0]["state"], "done", "{rows:?}");
    assert_eq!(rows[0]["ok"], false, "{rows:?}");
    assert_eq!(rows[0]["out_of_scope"], json!(["outside.txt"]), "{rows:?}");
}

#[test]
fn discard_removes_the_worktree_and_branch() {
    let s = Session::new();
    let repo = s.repo();
    let script = s.script("discard", json!([[{"sleep_ms": 60000}]]));
    s.config(&script, "");
    s.start("fix-11", "discard me", &repo, &[]);
    let worktree = s.task("fix-11")["task"]["worktree"].as_str().unwrap().to_string();
    let (_, _, err) = s.out(&["task", "discard", "fix-11"]);
    let error: Value = serde_json::from_str(err.trim()).unwrap();
    assert_eq!(error["error"]["kind"], "running");
    s.json(&["task", "cancel", "fix-11"]);
    let discarded = s.json(&["task", "discard", "fix-11"]);
    assert_eq!(discarded["state"], "discarded", "{discarded}");
    assert_eq!(discarded["removed"], true, "{discarded}");
    assert!(!Path::new(&worktree).exists(), "the worktree is gone");
    assert!(git(&repo, &["branch", "--list", "task/fix-11"]).trim().is_empty(), "the branch is gone");
}

#[test]
fn a_taken_or_bad_name_and_a_folder_without_git_are_refused() {
    let s = Session::new();
    let repo = s.repo();
    let script = s.script("names", json!([[{"sleep_ms": 60000}]]));
    s.config(&script, "");
    let first_brief = s.brief("first", "first");
    let (_, _, err) = s.out(&["task", "start", "Bad", "--brief", &first_brief, "--in", repo.to_str().unwrap()]);
    let error: Value = serde_json::from_str(err.trim()).unwrap();
    assert_eq!(error["error"]["kind"], "invalid_params", "{error}");

    s.start("same", "first", &repo, &[]);
    let (_, _, err) = s.out(&["task", "start", "same", "--brief", &first_brief, "--in", repo.to_str().unwrap()]);
    let error: Value = serde_json::from_str(err.trim()).unwrap();
    assert_eq!(error["error"]["kind"], "name_taken", "{error}");

    let plain = s.root.join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    let brief = s.brief("plain", "no repo");
    let (_, _, err) = s.out(&["task", "start", "plain-1", "--brief", &brief, "--in", plain.to_str().unwrap()]);
    let error: Value = serde_json::from_str(err.trim()).unwrap();
    assert_eq!(error["error"]["kind"], "not_a_repo", "{error}");
    s.json(&["task", "cancel", "same"]);
}

/// Every other test names its worker in the config, which is how this hid: with
/// no config the worker comes from PATH, and on Windows npm leaves a shell script
/// called `opencode` beside the real program. Finding the worker and asking its
/// version both come before the check for a repo, so `not_a_repo` proves them.
#[test]
fn a_worker_on_path_is_found_past_a_shell_script_of_the_same_name() {
    let s = Session::new();
    let bin = s.root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::copy(TEST_WORKER, bin.join(format!("opencode{}", std::env::consts::EXE_SUFFIX))).unwrap();
    if cfg!(windows) {
        std::fs::write(bin.join("opencode"), "#!/bin/sh\nexit 1\n").unwrap();
    }
    let mut path = vec![bin];
    path.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()));
    let plain = s.root.join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    let brief = s.brief("found", "go");
    let out = s
        .cmd()
        .env("PATH", std::env::join_paths(path).unwrap())
        .args(["task", "start", "found", "--brief", &brief, "--in", plain.to_str().unwrap()])
        .output()
        .unwrap();
    let said = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(said.contains("not_a_repo"), "{said}");
}

#[test]
fn a_broken_record_costs_one_task() {
    let s = Session::new();
    let repo = s.repo();
    let mut actions = vec![];
    actions.extend(say(&stream("ses_good", "good")));
    let script = s.script("broken", json!([actions]));
    s.config(&script, "");
    s.start("good", "fine", &repo, &[]);
    let (exit, _, _) = s.wait(&["good"], &["--timeout", "30"]);
    assert_eq!(exit, 0);
    s.out(&["server", "stop"]);
    let broken = s.state().join("tasks").join("broken");
    std::fs::create_dir_all(&broken).unwrap();
    std::fs::write(broken.join("record.json"), "not json at all").unwrap();
    let list = s.json(&["task", "ls", "--all"]);
    let names: Vec<&str> = list["tasks"].as_array().unwrap().iter().filter_map(|t| t["task"].as_str()).collect();
    assert!(names.contains(&"good"), "{list}");
    assert!(
        list["problems"].as_array().unwrap().iter().any(|p| p.as_str().unwrap_or_default().contains("broken")),
        "{list}"
    );
    assert!(broken.join("record.broken.json").is_file());
}

#[test]
fn a_wait_can_reach_the_core_before_the_start() {
    let s = Session::new();
    let repo = s.repo();
    let mut actions = vec![];
    actions.extend(say(&stream("ses_late", "late but done")));
    let script = s.script("late", json!([actions]));
    s.config(&script, "");
    let wait = s
        .cmd()
        .args(["task", "wait", "fix-14", "--timeout", "20"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(500));
    s.start("fix-14", "arrive late", &repo, &[]);
    let out = wait.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("\"state\":\"done\""), "{text}");

    let out = s.output(&["task", "wait", "nope", "--grace", "1", "--timeout", "10"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(1), "{stdout} {}", String::from_utf8_lossy(&out.stderr));
    let summary: Value = serde_json::from_str(stdout.lines().last().unwrap()).unwrap();
    assert_eq!(summary["summary"]["unknown"], json!(["nope"]), "{summary}");
}
