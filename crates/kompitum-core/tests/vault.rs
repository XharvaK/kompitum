//! `kom vault` end to end: a bare repository as the remote and one clone per
//! agent, all under this test's temp folder. No network, no core.
//! The same tests run on Windows here and on Linux in CI.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

const BIN: &str = env!("CARGO_BIN_EXE_kompitum-core");

struct Fixture {
    root: PathBuf,
    opus: PathBuf,
    manager: PathBuf,
    session: String,
}

impl Fixture {
    fn new() -> Fixture {
        static N: AtomicU32 = AtomicU32::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("kom-vault-test-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let remote = root.join("remote.git");
        let opus = root.join("opus");
        let manager = root.join("manager");

        git(&root, &["init", "-q", "--bare", "-b", "main", remote.to_str().unwrap()]);
        git(&root, &["init", "-q", "-b", "main", opus.to_str().unwrap()]);
        configure(&opus);
        std::fs::write(opus.join("README.md"), "# Test vault\n").unwrap();
        commit(&opus, "seed");
        git(&opus, &["remote", "add", "origin", remote.to_str().unwrap()]);
        git(&opus, &["push", "-q", "-u", "origin", "main"]);
        git(&root, &["clone", "-q", "-c", "core.autocrlf=false", remote.to_str().unwrap(), manager.to_str().unwrap()]);
        configure(&manager);

        Fixture { session: format!("vault-{}-{n}", std::process::id()), root, opus, manager }
    }

    fn vault(&self, who: &str) -> &Path {
        if who == "opus" { &self.opus } else { &self.manager }
    }

    fn cmd(&self, who: &str) -> Command {
        let mut cmd = Command::new(BIN);
        cmd.current_dir(&self.root)
            .env("KOMPITUM_VAULT", self.vault(who))
            .env("KOMPITUM_AGENT", who)
            .env("KOMPITUM_SESSION", format!("{}-{who}", self.session))
            .env("XDG_STATE_HOME", self.root.join("state"))
            .env("XDG_CONFIG_HOME", self.root.join("config"))
            .env_remove("KOMPITUM_WORKER")
            .env_remove("KOMPITUM_SOCKET")
            .env_remove("KOMPITUM_TERMINAL_ID")
            .env_remove("KOMPITUM_SCOPE")
            .stdin(Stdio::null());
        kompitum_core::platform::no_console(&mut cmd);
        cmd
    }

    fn out(&self, who: &str, args: &[&str]) -> Output {
        self.cmd(who).args(args).output().unwrap()
    }

    fn input(&self, who: &str, args: &[&str], input: &str) -> Output {
        use std::io::Write;
        let mut cmd = self.cmd(who);
        cmd.args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = cmd.spawn().unwrap();
        let mut stdin = child.stdin.take().unwrap();
        let input = input.as_bytes().to_vec();
        let feeder = std::thread::spawn(move || drop(stdin.write_all(&input)));
        let out = child.wait_with_output().unwrap();
        feeder.join().unwrap();
        out
    }

    fn json(&self, who: &str, args: &[&str]) -> Value {
        let out = self.out(who, &[args, &["--json"]].concat());
        assert!(
            out.status.success(),
            "kom {args:?} as {who} failed: {}\n{}",
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout)
        );
        serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
            panic!("kom {args:?} as {who}: bad json ({e}): {}", String::from_utf8_lossy(&out.stdout))
        })
    }

    fn json_input(&self, who: &str, args: &[&str], input: &str) -> Value {
        let out = self.input(who, &[args, &["--json"]].concat(), input);
        assert!(
            out.status.success(),
            "kom {args:?} as {who} failed: {}\n{}",
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout)
        );
        serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
            panic!("kom {args:?} as {who}: bad json ({e}): {}", String::from_utf8_lossy(&out.stdout))
        })
    }

    fn failed(&self, who: &str, args: &[&str], stdin: Option<&str>) -> Value {
        let out = match stdin {
            Some(input) => self.input(who, &[args, &["--json"]].concat(), input),
            None => self.out(who, &[args, &["--json"]].concat()),
        };
        assert!(!out.status.success(), "kom {args:?} as {who} unexpectedly succeeded");
        serde_json::from_slice(&out.stderr).unwrap_or_else(|e| {
            panic!("kom {args:?} as {who}: bad error json ({e}): {}", String::from_utf8_lossy(&out.stderr))
        })
    }

    fn commit(&self, who: &str, message: &str) -> String {
        let dir = self.vault(who);
        git(dir, &["pull", "-q", "--rebase"]);
        git(dir, &["add", "."]);
        git(dir, &["commit", "-q", "-m", message]);
        git(dir, &["push", "-q"]);
        git(dir, &["rev-parse", "HEAD"]).trim().to_string()
    }

    fn write(&self, who: &str, path: &str, text: &str) {
        let full = self.vault(who).join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, text).unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        for _ in 0..5 {
            match std::fs::remove_dir_all(&self.root) {
                Ok(()) => return,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
                Err(_) => std::thread::sleep(std::time::Duration::from_millis(100)),
            }
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn configure(dir: &Path) {
    for (key, value) in [
        ("user.name", "Test"),
        ("user.email", "test@example.org"),
        ("commit.gpgsign", "false"),
        ("core.autocrlf", "false"),
    ] {
        git(dir, &["config", key, value]);
    }
}

fn git(dir: &Path, args: &[&str]) -> String {
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    kompitum_core::platform::no_console(&mut cmd);
    let out = cmd.output().unwrap();
    assert!(out.status.success(), "git {args:?} in {}: {}", dir.display(), String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn commit(dir: &Path, message: &str) {
    git(dir, &["add", "."]);
    git(dir, &["commit", "-q", "-m", message]);
}

fn note(from: &str, to: &str, kind: &str, date: &str, title: &str, body: &str) -> String {
    format!("---\nfrom: {from}\nto: [{to}]\nkind: {kind}\ndate: {date}\ntitle: {title}\n---\n\n# {title}\n\n{body}\n")
}

#[test]
fn a_post_round_trips_through_the_other_clone() {
    let f = Fixture::new();
    let posted = f.json_input(
        "opus",
        &[
            "vault",
            "post",
            "--to",
            "manager",
            "--kind",
            "request",
            "--title",
            "The key",
            "--body-file",
            "-",
            "--project",
            "myapp",
        ],
        "Is it in the vault?",
    );
    let path = posted["path"].as_str().unwrap().to_string();
    assert!(path.starts_with("opus/to/manager/") && path.ends_with("_the-key.md"), "{path}");
    assert_eq!(posted["pushed"], true, "{posted}");
    assert!(posted["said"].as_str().unwrap().contains("Committed and pushed."), "{posted}");

    let open = f.json("manager", &["vault", "open"]);
    assert_eq!(open["notes"][0]["path"], path);
    assert_eq!(open["notes"][0]["from"], "opus");
    assert_eq!(open["notes"][0]["to"], "manager");
    assert_eq!(open["notes"][0]["kind"], "request");
    assert_eq!(open["notes"][0]["status"], "open");
    assert_eq!(open["notes"][0]["title"], "The key");
    assert_eq!(open["notes"][0]["project"], "myapp");
    assert!(open["notes"][0].get("body").is_none(), "body is for read and thread only");

    let read = f.json("manager", &["vault", "read", &path]);
    assert_eq!(read["note"]["path"], path);
    assert!(read["note"]["body"].as_str().unwrap().contains("Is it in the vault?"));
    assert_eq!(read["note"]["cut"], false);
    let readers = read["note"]["read_by"].as_array().unwrap();
    assert_eq!(readers[0]["agent"], "manager");
    assert!(readers[0]["at"].as_str().unwrap().contains('+'));

    // A request read is still open until answered or done.
    assert_eq!(f.json("manager", &["vault", "open"])["notes"][0]["path"], path);
}

#[test]
fn a_note_in_the_python_fixture_format_reads_back_the_same() {
    let f = Fixture::new();
    let text = "---\r\nfrom: manager\r\nto: [opus]\r\nkind: request\r\ntitle: \"Key: rotate\"\r\n\
                project: myapp\r\n---\r\n\r\n# A note from manager\r\n\r\nIs the key in the vault?\r\n";
    f.write("manager", "manager/to/opus/2026-10-19_1208_key.md", text);
    f.commit("manager", "manager: request key");

    let open = f.json("opus", &["vault", "open"]);
    let note = &open["notes"][0];
    assert_eq!(note["path"], "manager/to/opus/2026-10-19_1208_key.md");
    assert_eq!(note["from"], "manager");
    assert_eq!(note["to"], "opus");
    assert_eq!(note["kind"], "request");
    assert_eq!(note["status"], "open");
    assert_eq!(note["title"], "Key: rotate");
    assert_eq!(note["project"], "myapp");
    assert_eq!(note["date"], serde_json::Value::Null);

    let read = f.json("opus", &["vault", "read", "manager/to/opus/2026-10-19_1208_key.md"]);
    assert_eq!(read["note"]["title"], "Key: rotate");
    assert!(read["note"]["body"].as_str().unwrap().contains("Is the key in the vault?"));
}

#[test]
fn a_private_message_is_refused_and_nothing_is_written() {
    let f = Fixture::new();
    let token = ["gh", "p_", "aB3dE6gH9jK2mN5pQ8sT1vW4"].concat();
    let error = f.failed(
        "opus",
        &[
            "vault",
            "post",
            "--to",
            "manager",
            "--kind",
            "report",
            "--title",
            "Token",
            "--body",
            &format!("the token {token}"),
        ],
        None,
    );
    let message = error["error"]["message"].as_str().unwrap();
    assert!(message.contains("a GitHub token"), "{message}");
    assert!(message.contains("Nothing was written"), "{message}");
    assert!(!f.opus.join("opus/to/manager").exists(), "no note may be left behind");
}

#[test]
fn open_is_closed_by_a_reply_or_the_senders_mark() {
    let f = Fixture::new();
    f.write(
        "manager",
        "manager/to/opus/a.md",
        &note("manager", "opus", "request", "2026-11-01T09:00+03:00", "One", "First."),
    );
    f.write(
        "manager",
        "manager/to/opus/b.md",
        &note("manager", "opus", "request", "2026-11-02T09:00+03:00", "Two", "Second."),
    );
    f.commit("manager", "manager: two requests");
    assert_eq!(f.json("opus", &["vault", "open"])["notes"].as_array().unwrap().len(), 2);

    let posted = f.json(
        "opus",
        &[
            "vault",
            "post",
            "--to",
            "manager",
            "--kind",
            "answer",
            "--title",
            "Both",
            "--body",
            "Done.",
            "--re",
            "manager/to/opus/a.md",
            "--re",
            "manager/to/opus/b.md",
        ],
    );
    let answer = posted["path"].as_str().unwrap();
    assert!(answer.starts_with("opus/to/manager/"));
    assert_eq!(f.json("opus", &["vault", "open"])["notes"], serde_json::json!([]), "my reply closed both");

    // The sender's mark closes a note for its recipient too.
    f.write(
        "manager",
        "manager/to/opus/c.md",
        &note("manager", "opus", "request", "2026-11-03T09:00+03:00", "Three", "Third."),
    );
    f.commit("manager", "manager: a third");
    assert_eq!(f.json("opus", &["vault", "open"])["notes"][0]["path"], "manager/to/opus/c.md");
    let marked = f.json("manager", &["vault", "mark", "manager/to/opus/c.md", "answered"]);
    assert_eq!(marked["pushed"], true);
    assert_eq!(f.json("opus", &["vault", "open"])["notes"], serde_json::json!([]));
}

#[test]
fn read_marks_close_a_decision_and_done_closes_a_question() {
    let f = Fixture::new();
    f.write(
        "manager",
        "manager/to/opus/decision.md",
        &note("manager", "opus", "decision", "2026-11-04T18:26+03:00", "One vault", "A decision."),
    );
    f.write(
        "manager",
        "manager/to/opus/question.md",
        &note("manager", "opus", "question", "2026-11-04T18:30+03:00", "A question", "What now?"),
    );
    f.commit("manager", "manager: a decision and a question");
    assert_eq!(f.json("opus", &["vault", "open"])["notes"].as_array().unwrap().len(), 2);

    let read = f.json("opus", &["vault", "read", "manager/to/opus/decision.md"]);
    assert_eq!(read["note"]["read_by"][0]["agent"], "opus");
    let open = f.json("opus", &["vault", "open"]);
    let paths: Vec<&str> = open["notes"].as_array().unwrap().iter().map(|n| n["path"].as_str().unwrap()).collect();
    assert_eq!(paths, ["manager/to/opus/question.md"], "a read decision leaves the list");
    assert!(f.vault("opus").join("opus/reads.jsonl").exists());

    // Reading again leaves no second mark.
    f.json("opus", &["vault", "read", "manager/to/opus/decision.md"]);
    let marks = std::fs::read_to_string(f.vault("opus").join("opus/reads.jsonl")).unwrap();
    assert_eq!(marks.lines().filter(|line| line.contains("decision.md")).count(), 1);

    let done = f.json("opus", &["vault", "done", "manager/to/opus/question.md", "--why", "answered elsewhere"]);
    assert_eq!(done["pushed"], true, "{done}");
    assert_eq!(f.json("opus", &["vault", "open"])["notes"], serde_json::json!([]));
    let marks = std::fs::read_to_string(f.vault("opus").join("opus/reads.jsonl")).unwrap();
    assert!(marks.contains("\"done_at\""), "{marks}");
    assert!(marks.contains("answered elsewhere"), "{marks}");

    // The sender sees both marks without being woken.
    let found = f.json("manager", &["vault", "find", "question"]);
    let question =
        found["notes"].as_array().unwrap().iter().find(|n| n["path"] == "manager/to/opus/question.md").unwrap();
    assert_eq!(question["done"]["agent"], "opus");
    assert_eq!(question["done"]["why"], "answered elsewhere");
}

#[test]
fn thread_is_oldest_first_and_keeps_the_last_n() {
    let f = Fixture::new();
    f.write(
        "manager",
        "manager/to/opus/a.md",
        &note("manager", "opus", "report", "2026-11-01T09:00+03:00", "A", "First."),
    );
    f.commit("manager", "manager: a");
    f.write(
        "manager",
        "manager/to/opus/b.md",
        &note("manager", "opus", "report", "2026-11-02T09:00+03:00", "B", "Second."),
    );
    f.commit("manager", "manager: b");
    f.write(
        "opus",
        "opus/to/manager/c.md",
        &note("opus", "manager", "answer", "2026-11-03T09:00+03:00", "C", "Third."),
    );
    f.commit("opus", "opus: c");

    let thread = f.json("opus", &["vault", "thread", "manager", "--limit", "2"]);
    let paths: Vec<&str> = thread["notes"].as_array().unwrap().iter().map(|n| n["path"].as_str().unwrap()).collect();
    assert_eq!(paths, ["manager/to/opus/b.md", "opus/to/manager/c.md"]);
    assert_eq!(thread["notes"][0]["body"], "# B\n\nSecond.\n");
    assert_eq!(thread["notes"][0]["cut"], false);
}

#[test]
fn changes_lists_what_came_after_the_last_commit_it_saw() {
    let f = Fixture::new();
    let first = f.json("opus", &["vault", "changes"]);
    assert_eq!(first["notes"], serde_json::json!([]), "the first run lists none");

    let posted =
        f.json("manager", &["vault", "post", "--to", "opus", "--kind", "report", "--title", "New", "--body", "Fresh."]);
    let path = posted["path"].as_str().unwrap();

    let changes = f.json("opus", &["vault", "changes"]);
    assert_eq!(changes["notes"][0]["path"], path);
    assert!(changes["notes"][0].get("body").is_none());
    assert_eq!(f.json("opus", &["vault", "changes"])["notes"], serde_json::json!([]), "and it remembers");

    let state = f.root.join("state").join("kompitum-core").join(format!("{}-opus", f.session));
    let seen = std::fs::read_to_string(state.join("vault-seen-opus")).unwrap();
    assert_eq!(seen.trim().len(), 40, "the newest commit is remembered");
}

#[test]
fn an_unpushed_note_is_counted_when_the_remote_refuses() {
    let f = Fixture::new();
    // Pulls still come from origin; only the push goes to a place that does not exist.
    git(&f.opus, &["config", "remote.origin.pushurl", f.root.join("missing.git").to_str().unwrap()]);
    let posted = f.json(
        "opus",
        &["vault", "post", "--to", "manager", "--kind", "report", "--title", "Stuck", "--body", "Still here."],
    );
    assert_eq!(posted["pushed"], false, "{posted}");
    assert!(posted["said"].as_str().unwrap().contains("Committed"), "{posted}");

    let open = f.json("opus", &["vault", "open"]);
    assert!(open["unpushed"].as_u64().unwrap() >= 1, "{open}");
    let problems = open["problems"].as_array().unwrap();
    assert!(problems.iter().any(|p| p.as_str().unwrap().contains("unpushed")), "{open}");
}

#[test]
fn as_overrides_the_reader_and_the_writer() {
    let f = Fixture::new();
    f.json(
        "manager",
        &["vault", "post", "--to", "opus", "--kind", "request", "--title", "To Opus", "--body", "Only you."],
    );
    assert_eq!(f.json("opus", &["vault", "open"])["notes"].as_array().unwrap().len(), 1);
    assert_eq!(
        f.json("opus", &["vault", "open", "--as", "manager"])["notes"],
        serde_json::json!([]),
        "as manager, the note to opus is not mine"
    );
    let posted = f.json(
        "opus",
        &[
            "vault",
            "post",
            "--as",
            "manager",
            "--to",
            "opus",
            "--kind",
            "report",
            "--title",
            "From Manager",
            "--body",
            "Hi.",
        ],
    );
    assert!(posted["path"].as_str().unwrap().starts_with("manager/to/opus/"), "{}", posted["path"]);
}

#[test]
fn a_worker_may_look_but_not_write() {
    let f = Fixture::new();
    let mut cmd = f.cmd("opus");
    cmd.env("KOMPITUM_WORKER", "1");
    let out = cmd
        .args(["vault", "post", "--to", "manager", "--kind", "report", "--title", "T", "--body", "B"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let error: Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(error["error"]["kind"], "worker_refused");
}
