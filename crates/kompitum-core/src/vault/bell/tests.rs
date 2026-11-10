//! Tests for the vault bell: the Python bell's tests ported (`kompitum-mcp/tests/test_kompitum_bell.py`),
//! plus what the brief adds. Real git runs in temporary repositories; the POST is a fake, except one test that
//! talks to a local server.

use super::*;
use std::cell::RefCell;
use std::io::{BufRead, BufReader, Read};
use std::rc::Rc;

const NOW: f64 = 1_700_000_000.0;

fn now() -> f64 {
    NOW
}

/// Every POST the fake webhook received (URL, token, body), with the answer it gives.
#[derive(Clone, Default)]
struct Calls(Rc<RefCell<Vec<(String, String, String)>>>);

impl Calls {
    fn answering(&self, answer: Result<u16, String>) -> PostFn {
        let seen = self.0.clone();
        Box::new(move |url, token, body| {
            seen.borrow_mut().push((url.into(), token.into(), String::from_utf8_lossy(body).into_owned()));
            answer.clone()
        })
    }

    fn count(&self) -> usize {
        self.0.borrow().len()
    }

    fn body(&self, index: usize) -> Value {
        serde_json::from_str(&self.0.borrow()[index].2).unwrap()
    }

    fn new_notes(&self, index: usize) -> Vec<String> {
        self.body(index)["new_notes"].as_array().unwrap().iter().map(|n| n.as_str().unwrap().to_string()).collect()
    }

    fn token(&self, index: usize) -> String {
        self.0.borrow()[index].1.clone()
    }
}

fn names(events: &[Event]) -> Vec<&'static str> {
    events.iter().map(|event| event.action.name()).collect()
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn git_in(dir: &Path, args: &[&str]) -> String {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(dir).args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    platform::no_console(&mut cmd);
    let out = cmd.output().expect("git runs");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn configure(dir: &Path) {
    for (key, value) in [
        ("user.name", "Test"),
        ("user.email", "test@example.org"),
        ("commit.gpgsign", "false"),
        ("core.autocrlf", "false"),
    ] {
        git_in(dir, &["config", key, value]);
    }
}

/// A vault with a bare remote, the vault itself (what the bell watches) and a writer clone that adds notes.
struct Fixture {
    root: PathBuf,
    vault: PathBuf,
    writer: PathBuf,
    state: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Fixture {
        let root = std::env::temp_dir().join(format!("kom-bell-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let remote = root.join("remote.git");
        let vault = root.join("vault");
        let writer = root.join("writer");
        git_in(&root, &["init", "-q", "--bare", "-b", "main", &path_text(&remote)]);
        git_in(&root, &["init", "-q", "-b", "main", &path_text(&vault)]);
        configure(&vault);
        std::fs::write(vault.join("README.md"), "# Vault\n").unwrap();
        git_in(&vault, &["add", "."]);
        git_in(&vault, &["commit", "-q", "-m", "seed"]);
        git_in(&vault, &["remote", "add", "origin", &path_text(&remote)]);
        git_in(&vault, &["push", "-q", "-u", "origin", "main"]);
        git_in(&root, &["clone", "-q", &path_text(&remote), &path_text(&writer)]);
        configure(&writer);
        let state = root.join("bell-state.json");
        Fixture { root, vault, writer, state }
    }

    /// Writes the given notes in the writer clone, commits and pushes them; the new head.
    fn push(&self, paths: &[&str]) -> String {
        for path in paths {
            let file = self.writer.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(&file, format!("# {path}\n")).unwrap();
        }
        git_in(&self.writer, &["add", "."]);
        git_in(&self.writer, &["commit", "-q", "-m", "notes"]);
        git_in(&self.writer, &["push", "-q"]);
        self.writer_head()
    }

    fn writer_head(&self) -> String {
        git_in(&self.writer, &["rev-parse", "HEAD"]).trim().to_string()
    }

    fn env(&self, targets: &[&str], reference: Option<&str>) -> BTreeMap<String, String> {
        let mut env = BTreeMap::new();
        env.insert("KOMPITUM_VAULT".to_string(), path_text(&self.vault));
        env.insert("KOMPITUM_BELL_TARGETS".to_string(), targets.join(","));
        env.insert("KOMPITUM_BELL_POLL_SECONDS".to_string(), "60".to_string());
        env.insert("KOMPITUM_BELL_STATE".to_string(), path_text(&self.state));
        if let Some(reference) = reference {
            env.insert("KOMPITUM_BELL_REF".to_string(), reference.to_string());
        }
        for name in targets {
            let key = env_key(name);
            env.insert(format!("KOMPITUM_BELL_{key}_URL"), format!("https://hooks.invalid/{name}"));
            env.insert(format!("KOMPITUM_BELL_{key}_TOKEN"), format!("shared-secret-{name}"));
        }
        env
    }

    fn bell(&self, post: PostFn) -> Bell {
        self.bell_for(&["manager"], Some("origin/main"), post)
    }

    fn bell_for(&self, targets: &[&str], reference: Option<&str>, post: PostFn) -> Bell {
        let config = load_config(&self.env(targets, reference), &self.root).expect("the settings load");
        Bell::new(config, now, post)
    }

    fn state_json(&self) -> Value {
        serde_json::from_str(&std::fs::read_to_string(&self.state).unwrap()).unwrap()
    }

    fn mark(&self, name: &str) -> Mark {
        read_state(&self.state).0.get(name).cloned().expect("a mark")
    }
}

fn drive_with(
    f: &Fixture,
    env: &BTreeMap<String, String>,
    options: Options,
    post: PostFn,
) -> (i32, Vec<String>, Vec<String>) {
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = drive(env, &f.root, &options, post, &mut |line| out.push(line.to_string()), &mut |line| {
        err.push(line.to_string())
    });
    (code, out, err)
}

fn once() -> Options {
    Options { once: true, ..Options::default() }
}

// --- The tick: what rings, what does not, and the mark ---------------------------------------------------

#[test]
fn first_tick_only_watches_and_never_rings_the_backlog() {
    let f = Fixture::new("first-tick");
    f.push(&["opus/to/manager/2026-10-01_0000_old.md"]);
    let calls = Calls::default();
    let events = f.bell(calls.answering(Ok(200))).tick().unwrap();
    assert_eq!(names(&events), ["watching"]);
    assert_eq!(events[0].target.as_deref(), Some("manager"));
    assert_eq!(calls.count(), 0);
    assert_eq!(f.mark("manager"), Mark { commit: f.writer_head(), rang_at: None, last_count: 0 });
}

#[test]
fn a_note_for_the_target_rings_with_the_body_and_the_header() {
    let f = Fixture::new("one-note");
    let calls = Calls::default();
    let mut bell = f.bell(calls.answering(Ok(200)));
    bell.tick().unwrap();
    let head = f.push(&["opus/to/manager/2026-11-01_0400_example.md"]);
    let events = bell.tick().unwrap();
    assert_eq!(names(&events), ["rang"]);
    assert_eq!(events[0].count, 1);
    assert_eq!(events[0].status, Some(Status::Http(200)));
    assert_eq!(calls.count(), 1);
    assert_eq!(calls.0.borrow()[0].0, "https://hooks.invalid/manager");
    assert_eq!(calls.token(0), "shared-secret-manager");
    assert_eq!(
        calls.0.borrow()[0].2,
        r#"{"source":"vault-doorbell","new_notes":["opus/to/manager/2026-11-01_0400_example.md"]}"#
    );
    assert_eq!(f.mark("manager"), Mark { commit: head, rang_at: Some(NOW), last_count: 1 });
}

#[test]
fn all_rings_but_the_targets_own_note_and_another_agents_do_not() {
    let f = Fixture::new("all-and-own");
    let calls = Calls::default();
    let mut bell = f.bell(calls.answering(Ok(200)));
    bell.tick().unwrap();
    f.push(&["thinker/to/all/2026-11-01_0401_example.md", "manager/to/manager/own.md", "opus/to/thinker/other.md"]);
    let events = bell.tick().unwrap();
    assert_eq!(names(&events), ["rang"]);
    assert_eq!(calls.new_notes(0), ["thinker/to/all/2026-11-01_0401_example.md"]);
}

#[test]
fn a_modified_file_does_not_ring() {
    let f = Fixture::new("modified");
    f.push(&["opus/to/manager/a.md"]);
    let calls = Calls::default();
    let mut bell = f.bell(calls.answering(Ok(200)));
    bell.tick().unwrap();
    std::fs::write(f.writer.join("opus/to/manager/a.md"), "changed\n").unwrap();
    git_in(&f.writer, &["add", "."]);
    git_in(&f.writer, &["commit", "-q", "-m", "change"]);
    git_in(&f.writer, &["push", "-q"]);
    let events = bell.tick().unwrap();
    assert_eq!(names(&events), ["nothing"]);
    assert_eq!(calls.count(), 0);
    assert_eq!(f.mark("manager").commit, f.writer_head());
}

#[test]
fn two_notes_ring_together_and_a_later_one_rings_alone_next_tick() {
    let f = Fixture::new("two-then-one");
    let calls = Calls::default();
    let mut bell = f.bell(calls.answering(Ok(200)));
    bell.tick().unwrap();
    f.push(&["opus/to/manager/a.md", "thinker/to/all/b.md"]);
    bell.tick().unwrap();
    assert_eq!(calls.count(), 1);
    assert_eq!(calls.new_notes(0), ["opus/to/manager/a.md", "thinker/to/all/b.md"]);
    f.push(&["opus/to/manager/c.md"]);
    bell.tick().unwrap();
    assert_eq!(calls.count(), 2);
    assert_eq!(calls.new_notes(1), ["opus/to/manager/c.md"]);
    assert_eq!(f.mark("manager").last_count, 1);
}

#[test]
fn a_note_moved_into_the_inbox_rings() {
    let f = Fixture::new("moved");
    f.push(&["opus/drafts/a.md"]);
    let calls = Calls::default();
    let mut bell = f.bell(calls.answering(Ok(200)));
    bell.tick().unwrap();
    std::fs::create_dir_all(f.writer.join("opus/to/manager")).unwrap();
    git_in(&f.writer, &["mv", "opus/drafts/a.md", "opus/to/manager/a.md"]);
    git_in(&f.writer, &["commit", "-q", "-m", "move"]);
    git_in(&f.writer, &["push", "-q"]);
    bell.tick().unwrap();
    assert_eq!(calls.new_notes(0), ["opus/to/manager/a.md"]);
}

#[test]
fn a_file_directly_in_to_does_not_ring() {
    let f = Fixture::new("direct-to");
    let calls = Calls::default();
    let mut bell = f.bell(calls.answering(Ok(200)));
    bell.tick().unwrap();
    f.push(&["opus/to/manager", "thinker/to/all"]);
    assert_eq!(names(&bell.tick().unwrap()), ["nothing"]);
    assert_eq!(calls.count(), 0);
}

#[test]
fn a_commit_with_no_kept_paths_moves_the_mark() {
    let f = Fixture::new("no-kept");
    let calls = Calls::default();
    let mut bell = f.bell(calls.answering(Ok(200)));
    bell.tick().unwrap();
    let head = f.push(&["opus/to/thinker/elsewhere.md"]);
    assert_eq!(names(&bell.tick().unwrap()), ["nothing"]);
    assert_eq!(calls.count(), 0);
    assert_eq!(f.mark("manager").commit, head);
}

#[test]
fn a_failed_ring_keeps_the_mark_and_is_retried() {
    let f = Fixture::new("failed-retry");
    f.bell(Calls::default().answering(Ok(200))).tick().unwrap();
    let watched = f.mark("manager").commit;
    f.push(&["opus/to/manager/a.md"]);
    let failing = Calls::default();
    let events = f.bell(failing.answering(Ok(500))).tick().unwrap();
    assert_eq!(names(&events), ["failed"]);
    assert_eq!((events[0].count, events[0].status.clone()), (1, Some(Status::Http(500))));
    assert_eq!(f.mark("manager").commit, watched);
    let retry = Calls::default();
    let events = f.bell(retry.answering(Ok(200))).tick().unwrap();
    assert_eq!(names(&events), ["rang"]);
    assert_eq!(retry.new_notes(0), ["opus/to/manager/a.md"]);
}

#[test]
fn a_redirect_is_not_a_2xx_so_the_mark_stays() {
    let f = Fixture::new("redirect");
    f.bell(Calls::default().answering(Ok(200))).tick().unwrap();
    let watched = f.mark("manager").commit;
    f.push(&["opus/to/manager/a.md"]);
    let events = f.bell(Calls::default().answering(Ok(302))).tick().unwrap();
    assert_eq!(events[0].status, Some(Status::Http(302)));
    assert_eq!(f.mark("manager").commit, watched);
}

#[test]
fn an_exception_keeps_the_mark_and_never_shows_its_text() {
    let f = Fixture::new("exception");
    f.bell(Calls::default().answering(Ok(200))).tick().unwrap();
    let watched = f.mark("manager").commit;
    f.push(&["opus/to/manager/a.md"]);
    let events = f.bell(Calls::default().answering(Err("curl exit 7".into()))).tick().unwrap();
    assert_eq!(names(&events), ["failed"]);
    assert_eq!(events[0].status, Some(Status::Label("curl exit 7".into())));
    assert!(!format!("{events:?}").contains("hooks.invalid"));
    assert_eq!(f.mark("manager").commit, watched);
    let retry = Calls::default();
    let events = f.bell(retry.answering(Ok(200))).tick().unwrap();
    assert_eq!(names(&events), ["rang"]);
    assert_eq!(retry.new_notes(0), ["opus/to/manager/a.md"]);
}

#[test]
fn an_unknown_mark_commit_rings_the_whole_inbox_flagged_before_it_moves() {
    // AU 2026-11-03 NB-2: a lost mark must not skip notes silently.
    let f = Fixture::new("unknown-mark");
    let calls = Calls::default();
    f.bell(calls.answering(Ok(200))).tick().unwrap();
    f.push(&["opus/to/manager/old.md", "manager/to/opus/own.md"]);
    let zeros = "0".repeat(40);
    let lost = format!(r#"{{"manager": {{"commit": "{zeros}", "rang_at": null, "last_count": 0}}}}"#);
    std::fs::write(&f.state, lost).unwrap();
    let head = f.push(&["opus/to/manager/a.md", "thinker/to/all/b.md"]);
    let events = f.bell(Calls::default().answering(Ok(500))).tick().unwrap();
    assert_eq!(names(&events), ["failed"]);
    assert_eq!(f.mark("manager").commit, zeros, "no ring, no move");
    let events = f.bell(calls.answering(Ok(200))).tick().unwrap();
    assert_eq!(names(&events), ["rang"]);
    assert!(events[0].message.contains("reset"));
    assert_eq!(calls.new_notes(0), ["opus/to/manager/a.md", "opus/to/manager/old.md", "thinker/to/all/b.md"]);
    assert_eq!(calls.body(0)["mark_reset"], Value::Bool(true));
    assert_eq!(f.mark("manager").commit, head);
}

#[test]
fn targets_are_independent() {
    let f = Fixture::new("independent");
    let manager = Calls::default();
    let thinker = Calls::default();
    let mut posts = (manager.answering(Ok(200)), thinker.answering(Ok(200)));
    let route: PostFn =
        Box::new(
            move |url, token, body| {
                if url.ends_with("/manager") { (posts.0)(url, token, body) } else { (posts.1)(url, token, body) }
            },
        );
    let mut bell = f.bell_for(&["manager", "thinker"], Some("origin/main"), route);
    let first = bell.tick().unwrap();
    assert_eq!(
        first.iter().map(|e| (e.target.clone().unwrap(), e.action.name())).collect::<Vec<_>>(),
        [("manager".to_string(), "watching"), ("thinker".to_string(), "watching")]
    );
    f.push(&["opus/to/manager/a.md"]);
    let second = bell.tick().unwrap();
    assert_eq!(
        second.iter().map(|e| (e.target.clone().unwrap(), e.action.name())).collect::<Vec<_>>(),
        [("manager".to_string(), "rang"), ("thinker".to_string(), "nothing")]
    );
    assert_eq!(thinker.count(), 0);
    f.push(&["opus/to/thinker/b.md"]);
    let third = bell.tick().unwrap();
    assert_eq!(
        third.iter().map(|e| (e.target.clone().unwrap(), e.action.name())).collect::<Vec<_>>(),
        [("manager".to_string(), "nothing"), ("thinker".to_string(), "rang")]
    );
    assert_eq!(thinker.new_notes(0), ["opus/to/thinker/b.md"]);
    assert_eq!(thinker.token(0), "shared-secret-thinker");
}

// --- The state file -------------------------------------------------------------------------------------

#[test]
fn the_state_file_is_written_whole_and_leaves_no_temp_file_behind() {
    let f = Fixture::new("state-write");
    f.bell(Calls::default().answering(Ok(200))).tick().unwrap();
    let state = f.state_json();
    assert_eq!(state["manager"]["commit"], Value::String(f.writer_head()));
    assert_eq!(state["manager"]["rang_at"], Value::Null);
    assert_eq!(state["manager"]["last_count"], Value::from(0));
    let leftovers = std::fs::read_dir(&f.root)
        .unwrap()
        .filter(|entry| entry.as_ref().unwrap().file_name().to_string_lossy().ends_with(".tmp"))
        .count();
    assert_eq!(leftovers, 0);
}

/// The Python bell kept its state private to its user; so does this one, rewrite after rewrite.
#[cfg(unix)]
#[test]
fn the_state_file_is_private_to_its_user() {
    use std::os::unix::fs::PermissionsExt;
    let state = fresh_state("private");
    for _ in 0..2 {
        write_state(&state, &State::new()).unwrap();
        assert_eq!(std::fs::metadata(&state).unwrap().permissions().mode() & 0o777, 0o600);
    }
}

#[test]
fn a_corrupt_state_file_is_an_error_once_and_is_survived() {
    let f = Fixture::new("corrupt");
    std::fs::write(&f.state, "not json").unwrap();
    let mut bell = f.bell(Calls::default().answering(Ok(200)));
    let first = bell.tick().unwrap();
    assert_eq!(names(&first), ["error", "watching"]);
    assert!(first[0].message.contains("corrupt"));
    assert!(f.state_json().get("manager").is_some());
    std::fs::write(&f.state, "[1, 2]").unwrap();
    let second = bell.tick().unwrap();
    assert_eq!(names(&second), ["watching"]);
}

// --- Settings ------------------------------------------------------------------------------------------

#[test]
fn configuration_errors_name_the_variable() {
    let f = Fixture::new("config-errors");
    let no_git = f.root.join("not-a-vault");
    std::fs::create_dir_all(&no_git).unwrap();
    let mut env = f.env(&["manager"], Some("origin/main"));
    env.insert("KOMPITUM_VAULT".into(), path_text(&no_git));
    assert!(load_config(&env, &f.root).unwrap_err().contains("KOMPITUM_VAULT"));
    let mut env = f.env(&["manager"], Some("origin/main"));
    env.remove("KOMPITUM_BELL_MANAGER_URL");
    assert!(load_config(&env, &f.root).unwrap_err().contains("KOMPITUM_BELL_MANAGER_URL"));
    for bad in ["14", "abc", "nan", "inf"] {
        let mut env = f.env(&["manager"], Some("origin/main"));
        env.insert("KOMPITUM_BELL_POLL_SECONDS".into(), bad.into());
        assert!(load_config(&env, &f.root).unwrap_err().contains("KOMPITUM_BELL_POLL_SECONDS"), "{bad}");
    }
}

#[test]
fn a_tilde_vault_path_is_expanded() {
    let f = Fixture::new("tilde");
    let mut env = f.env(&["manager"], Some("origin/main"));
    env.insert("KOMPITUM_VAULT".into(), "~/vault".into());
    assert_eq!(load_config(&env, &f.root).unwrap().vault, f.vault);
}

#[test]
fn a_worktree_git_file_counts_as_a_vault() {
    let f = Fixture::new("worktree");
    let folder = f.root.join("worktree");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join(".git"), "gitdir: somewhere\n").unwrap();
    let env: BTreeMap<String, String> = [
        ("KOMPITUM_VAULT", path_text(&folder)),
        ("KOMPITUM_BELL_TARGETS", "manager".to_string()),
        ("KOMPITUM_BELL_MANAGER_URL", "https://hooks.invalid/manager".to_string()),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect();
    assert_eq!(load_config(&env, &f.root).unwrap().vault, folder);
}

#[test]
fn a_target_name_becomes_its_variable_key() {
    let f = Fixture::new("target-key");
    let vault_text = path_text(&f.vault);
    let env: BTreeMap<String, String> = [
        ("KOMPITUM_VAULT", vault_text.as_str()),
        ("KOMPITUM_BELL_TARGETS", "manager-1"),
        ("KOMPITUM_BELL_MANAGER_1_URL", "https://hooks.invalid/manager-1"),
        ("KOMPITUM_BELL_MANAGER_1_SOURCE", "my-doorbell"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    let config = load_config(&env, &f.root).unwrap();
    assert_eq!(
        config.targets,
        [Target {
            name: "manager-1".into(),
            url: "https://hooks.invalid/manager-1".into(),
            token: String::new(),
            source: "my-doorbell".into(),
        }]
    );
}

#[test]
fn env_file_skips_comments_and_blanks_and_strips_one_pair_of_quotes() {
    let text = "# a comment\n\n\
                KOMPITUM_BELL_MANAGER_URL=\"https://hooks.invalid/hook?a=1\"\n\
                KOMPITUM_BELL_MANAGER_TOKEN='tok'\n\
                KOMPITUM_BELL_POLL_SECONDS=30\n\
                KOMPITUM_BELL_ODD=\"abc'\n\
                not a setting\n\
                KOMPITUM_BELL_REF=origin/main\n";
    let mut env = BTreeMap::from([("KOMPITUM_BELL_POLL_SECONDS".to_string(), "99".to_string())]);
    apply_env_file(&mut env, parse_env_file(text));
    assert_eq!(env["KOMPITUM_BELL_MANAGER_URL"], "https://hooks.invalid/hook?a=1");
    assert_eq!(env["KOMPITUM_BELL_MANAGER_TOKEN"], "tok");
    assert_eq!(env["KOMPITUM_BELL_POLL_SECONDS"], "99", "a variable already set keeps its value");
    assert_eq!(env["KOMPITUM_BELL_REF"], "origin/main");
    assert_eq!(env["KOMPITUM_BELL_ODD"], "\"abc'", "mismatched quotes are kept");
    assert!(!env.contains_key("not a setting"));
}

#[test]
fn an_unreadable_env_file_is_a_configuration_error() {
    let f = Fixture::new("env-missing");
    assert!(read_env_file(&f.root.join("missing.env")).is_err());
}

#[test]
fn legacy_limen_names_are_read_when_the_new_ones_are_missing() {
    let f = Fixture::new("legacy");
    let vault_text = path_text(&f.vault);
    let env: BTreeMap<String, String> = [
        ("KOMPITUM_VAULT", vault_text.as_str()),
        ("LIMEN_BELL_TARGETS", "manager"),
        ("LIMEN_BELL_MANAGER_URL", "https://hooks.invalid/legacy"),
        ("LIMEN_BELL_POLL_SECONDS", "30"),
        ("KOMPITUM_BELL_REF", ""),
        ("LIMEN_BELL_REF", "origin/main"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    let config = load_config(&env, &f.root).unwrap();
    assert_eq!(config.targets[0].url, "https://hooks.invalid/legacy");
    assert_eq!(config.poll, 30.0);
    assert_eq!(config.reference, "origin/main", "an empty new name counts as unset");
    std::fs::create_dir_all(f.root.join(".limen")).unwrap();
    std::fs::write(f.root.join(".limen").join("bell-state.json"), "{}").unwrap();
    assert_eq!(default_state_path(&f.root), f.root.join(".limen").join("bell-state.json"));
    std::fs::create_dir_all(f.root.join(".kompitum")).unwrap();
    std::fs::write(f.root.join(".kompitum").join("bell-state.json"), "{}").unwrap();
    assert_eq!(default_state_path(&f.root), f.root.join(".kompitum").join("bell-state.json"));
}

// --- The command: status, dry run, once, and the secrets ----------------------------------------------

#[test]
fn status_shows_the_marks_but_never_the_url_or_the_token() {
    let f = Fixture::new("status");
    f.bell(Calls::default().answering(Ok(200))).tick().unwrap();
    let env = f.env(&["manager"], Some("origin/main"));
    let (code, out, _) =
        drive_with(&f, &env, Options { status: true, ..Options::default() }, Calls::default().answering(Ok(200)));
    assert_eq!(code, 0);
    assert_eq!(out.len(), 1);
    assert!(!out[0].contains("hooks.invalid"));
    assert!(!out[0].contains("shared-secret-manager"));
    let status: Value = serde_json::from_str(&out[0]).unwrap();
    assert_eq!(status["vault"], Value::String(path_text(&f.vault)));
    assert_eq!(status["ref"], "origin/main");
    assert_eq!(status["poll_seconds"], 60);
    assert_eq!(status["targets"]["manager"]["url"], "set");
    assert_eq!(status["targets"]["manager"]["token"], "set");
    assert_eq!(status["targets"]["manager"]["commit"], Value::String(f.writer_head()));
    assert_eq!(status["targets"]["manager"]["rang_at"], Value::Null);
}

#[test]
fn status_resolves_the_upstream_when_no_ref_is_set() {
    let f = Fixture::new("status-upstream");
    let env = f.env(&["manager"], None);
    let (code, out, _) =
        drive_with(&f, &env, Options { status: true, ..Options::default() }, Calls::default().answering(Ok(200)));
    assert_eq!(code, 0);
    assert_eq!(serde_json::from_str::<Value>(&out[0]).unwrap()["ref"], "origin/main");
}

#[test]
fn dry_run_posts_nothing_and_writes_no_state() {
    let f = Fixture::new("dry-run");
    f.bell(Calls::default().answering(Ok(200))).tick().unwrap();
    f.push(&["opus/to/manager/a.md"]);
    let before = std::fs::read_to_string(&f.state).unwrap();
    let calls = Calls::default();
    let env = f.env(&["manager"], Some("origin/main"));
    let (code, out, _) =
        drive_with(&f, &env, Options { dry_run: true, ..Options::default() }, calls.answering(Ok(200)));
    assert_eq!(code, 0);
    assert_eq!(calls.count(), 0);
    assert_eq!(std::fs::read_to_string(&f.state).unwrap(), before);
    assert_eq!(out, ["would ring manager: 1 note", "opus/to/manager/a.md"]);
}

#[test]
fn once_exit_codes_and_lines_without_secrets() {
    let f = Fixture::new("once");
    f.bell(Calls::default().answering(Ok(200))).tick().unwrap();
    f.push(&["opus/to/manager/a.md"]);
    let env = f.env(&["manager"], Some("origin/main"));
    let mut seen = Vec::new();
    let (code, out, _) = drive_with(&f, &env, once(), Calls::default().answering(Ok(500)));
    seen.extend(out);
    assert_eq!(code, 1);
    assert!(seen.iter().any(|line| line.ends_with("bell manager: failed 1 note (HTTP 500)")), "{seen:?}");
    let (code, out, _) = drive_with(&f, &env, once(), Calls::default().answering(Ok(200)));
    assert_eq!(code, 0);
    assert!(out.iter().any(|line| line.ends_with("bell manager: rang 1 note (HTTP 200)")), "{out:?}");
    f.push(&["opus/to/manager/b.md"]);
    let bad_ref = f.env(&["manager"], Some("origin/missing"));
    let (code, out, _) = drive_with(&f, &bad_ref, once(), Calls::default().answering(Ok(200)));
    assert_eq!(code, 1);
    assert!(out.iter().any(|line| line.contains("error") && line.contains("origin/missing")), "{out:?}");
    let mut bad_poll = f.env(&["manager"], Some("origin/main"));
    bad_poll.insert("KOMPITUM_BELL_POLL_SECONDS".into(), "14".into());
    let (code, _, err) = drive_with(&f, &bad_poll, once(), Calls::default().answering(Ok(200)));
    assert_eq!(code, 2);
    assert!(err[0].starts_with("KOMPITUM_BELL_POLL_SECONDS"), "{err:?}");
    for line in seen.iter().chain(out.iter()) {
        assert!(!line.contains("hooks.invalid") && !line.contains("shared-secret-manager"));
    }
}

#[test]
fn a_url_or_token_a_failure_names_is_never_printed() {
    let f = Fixture::new("printed-secrets");
    f.bell(Calls::default().answering(Ok(200))).tick().unwrap();
    f.push(&["opus/to/manager/a.md"]);
    let env = f.env(&["manager"], Some("origin/main"));
    let leaky: PostFn = Box::new(|url, token, _| Err(format!("refused {url} with {token}")));
    let (code, out, _) = drive_with(&f, &env, once(), leaky);
    assert_eq!(code, 1);
    assert!(!out.join("\n").contains("hooks.invalid"));
    assert!(!out.join("\n").contains("shared-secret-manager"));
}

#[test]
fn main_exits_cleanly_when_another_bell_runs() {
    let f = Fixture::new("already-running");
    let env = f.env(&["manager"], Some("origin/main"));
    let held = acquire_lock(&f.state).unwrap();
    assert!(held.is_some());
    let (code, out, _) = drive_with(&f, &env, once(), Calls::default().answering(Ok(200)));
    assert_eq!(code, 0);
    assert!(out.join(" ").contains("already running"), "{out:?}");
    release_lock(held.unwrap());
}

// --- The POST and its curl -------------------------------------------------------------------------------

#[test]
fn the_curl_argv_never_carries_the_url_or_the_token() {
    let args = curl_args(Path::new("/var/tmp/kom-bell-test.curl"));
    let joined = args.join(" ");
    assert!(!joined.contains("hooks.invalid") && !joined.contains("shared-secret"), "{joined}");
    assert!(!args.iter().any(|arg| arg == "-L"), "a redirect is never followed");
    let at = args.iter().position(|arg| arg == "--config").expect("--config");
    assert_eq!(args[at + 1], "/var/tmp/kom-bell-test.curl");
    assert!(args.windows(2).any(|pair| pair == ["--data-binary", "@-"]));
    assert!(args.windows(2).any(|pair| pair == ["--write-out", "%{http_code}"]));
}

#[test]
fn the_curl_config_holds_the_url_and_both_headers() {
    let text = curl_config("https://hooks.invalid/manager", "shared-secret-manager");
    assert!(text.contains("url = \"https://hooks.invalid/manager\"\n"));
    assert!(text.contains("header = \"Content-Type: application/json\"\n"));
    assert!(text.contains("header = \"Authorization: Bearer shared-secret-manager\"\n"));
    assert!(!curl_config("https://hooks.invalid/manager", "").contains("Authorization"));
}

#[test]
fn curl_post_sends_a_json_bearer_post_with_the_body() {
    use std::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://127.0.0.1:{}/hook", listener.local_addr().unwrap().port());
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut request = String::new();
        let mut length = 0usize;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if line == "\r\n" || line.is_empty() {
                break;
            }
            if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                length = value.trim().parse().unwrap();
            }
            request.push_str(&line);
        }
        let mut body = vec![0u8; length];
        reader.read_exact(&mut body).unwrap();
        stream.write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
        format!("{request}\n{}", String::from_utf8(body).unwrap())
    });
    let body = br#"{"source":"vault-doorbell","new_notes":["opus/to/manager/a.md"]}"#;
    assert_eq!(curl_post(&url, "tok", body), Ok(204));
    let seen = server.join().unwrap();
    let lower = seen.to_ascii_lowercase();
    assert!(seen.starts_with("POST /hook HTTP/1.1"), "{seen}");
    assert!(lower.contains("content-type: application/json"), "{seen}");
    assert!(lower.contains("authorization: bearer tok"), "{seen}");
    assert!(seen.ends_with(std::str::from_utf8(body).unwrap()), "{seen}");
}

// --- The single-instance lock ---------------------------------------------------------------------------

fn lock_file(state: &Path) -> PathBuf {
    lock_path(state)
}

fn write_lock(state: &Path, held: Value) {
    std::fs::write(lock_file(state), held.to_string()).unwrap();
}

fn fresh_state(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("kom-bell-lock-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root.join("bell-state.json")
}

/// A child process that has exited: its pid is dead.
fn exited_pid() -> u32 {
    let mut child = if cfg!(windows) {
        Command::new("cmd").args(["/C", "exit 0"]).spawn().unwrap()
    } else {
        Command::new("true").spawn().unwrap()
    };
    let pid = child.id();
    child.wait().unwrap();
    pid
}

/// A process that stays up for a while, killed when dropped.
#[cfg(target_os = "linux")]
struct Sleeper(std::process::Child);

#[cfg(target_os = "linux")]
impl Sleeper {
    fn plain() -> Sleeper {
        Sleeper(Command::new("sleep").arg("30").spawn().unwrap())
    }

    /// Its argument list names a bell, as `sh -c '...' kom vault bell` does.
    fn bell_like() -> Sleeper {
        Sleeper(Command::new("sh").args(["-c", "sleep 30; :", "kom", "vault", "bell"]).spawn().unwrap())
    }

    fn pid(&self) -> u32 {
        self.0.id()
    }
}

#[cfg(target_os = "linux")]
impl Drop for Sleeper {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn a_second_instance_is_refused_while_the_first_holds_the_lock() {
    let state = fresh_state("second");
    let first = acquire_lock(&state).unwrap();
    assert!(first.is_some());
    assert!(acquire_lock(&state).unwrap().is_none());
    release_lock(first.unwrap());
    let again = acquire_lock(&state).unwrap();
    assert!(again.is_some());
    release_lock(again.unwrap());
}

#[test]
fn a_stale_lock_from_a_dead_process_is_taken_over() {
    let state = fresh_state("stale");
    write_lock(&state, json!({"pid": exited_pid()}));
    let lock = acquire_lock(&state).unwrap();
    assert!(lock.is_some());
    release_lock(lock.unwrap());
    assert!(!lock_file(&state).exists());
}

#[test]
fn a_corrupt_lock_is_taken_over() {
    let state = fresh_state("corrupt-lock");
    std::fs::write(lock_file(&state), "not json").unwrap();
    let lock = acquire_lock(&state).unwrap();
    assert!(lock.is_some());
    release_lock(lock.unwrap());
}

#[test]
fn a_new_lock_records_pid_and_boot_id() {
    let state = fresh_state("records");
    let lock = acquire_lock(&state).unwrap().unwrap();
    let (pid, boot) = parse_owner(&std::fs::read_to_string(lock_file(&state)).unwrap());
    assert_eq!(pid, Some(std::process::id()));
    assert_eq!(boot, platform::boot_id());
    release_lock(lock);
}

#[test]
fn a_lock_from_another_boot_is_stale_even_when_its_pid_lives() {
    assert!(!lock_is_live(Some("boot-a"), Some("boot-b"), true));
    assert!(lock_is_live(Some("boot-a"), Some("boot-a"), true));
    assert!(lock_is_live(None, Some("boot-b"), true));
    assert!(!lock_is_live(Some("boot-a"), Some("boot-a"), false));
}

#[test]
fn only_a_bell_command_line_counts_as_a_bell() {
    let args = |list: &[&str]| list.iter().map(|a| a.to_string()).collect::<Vec<_>>();
    assert!(is_bell_argv(&args(&["/usr/local/bin/kom", "vault", "bell", "--env-file", "bell.env"])));
    assert!(is_bell_argv(&args(&["python3", "/home/x/kompitum/scripts/kompitum_cli.py", "bell"])));
    assert!(!is_bell_argv(&args(&["sleep", "30"])));
    assert!(!is_bell_argv(&args(&["/usr/bin/kom", "vault", "open"])));
}

#[cfg(target_os = "linux")]
#[test]
fn a_lock_from_a_different_boot_is_taken_over_even_if_the_pid_is_alive_now() {
    if platform::boot_id().is_none() {
        return;
    }
    let state = fresh_state("other-boot");
    let live = Sleeper::bell_like();
    write_lock(&state, json!({"pid": live.pid(), "boot_id": "another-boot", "token": "x"}));
    let lock = acquire_lock(&state).unwrap();
    assert!(lock.is_some());
    release_lock(lock.unwrap());
}

#[cfg(target_os = "linux")]
#[test]
fn a_reused_pid_that_is_not_a_bell_is_taken_over() {
    let state = fresh_state("reused");
    let other = Sleeper::plain();
    let boot = platform::boot_id();
    for held in [json!({"pid": other.pid()}), json!({"pid": other.pid(), "boot_id": boot, "token": "x"})] {
        write_lock(&state, held.clone());
        let lock = acquire_lock(&state).unwrap();
        assert!(lock.is_some(), "{held}");
        release_lock(lock.unwrap());
    }
}

#[cfg(target_os = "linux")]
#[test]
fn a_live_bell_in_this_boot_still_blocks() {
    let state = fresh_state("live-bell");
    let live = Sleeper::bell_like();
    let boot = platform::boot_id();
    for held in [json!({"pid": live.pid(), "boot_id": boot, "token": "x"}), json!({"pid": live.pid()})] {
        write_lock(&state, held.clone());
        assert!(acquire_lock(&state).unwrap().is_none(), "{held}");
    }
}

// --- Local helpers: the env file and the state's shape --------------------------------------------------

#[test]
fn a_mark_without_a_commit_makes_the_state_corrupt() {
    assert!(parse_state(r#"{"manager": {"rang_at": 1.0}}"#).is_none());
    assert!(parse_state(r#"[1, 2]"#).is_none());
    let state = parse_state(r#"{"manager": {"commit": "abc", "rang_at": true, "last_count": -1}}"#).unwrap();
    assert_eq!(state["manager"], Mark { commit: "abc".into(), rang_at: None, last_count: 0 });
}

#[test]
fn only_notes_for_the_target_or_all_are_kept_and_they_are_sorted() {
    let listing = "thinker/to/all/b.md\0manager/to/manager/own.md\0opus/to/manager/a.md\0opus/to/thinker/c.md\0\
                   opus/to/manager\0opus/to/all/d.md";
    assert_eq!(kept("manager", listing), ["opus/to/all/d.md", "opus/to/manager/a.md", "thinker/to/all/b.md"]);
}

#[test]
fn the_log_line_forms_match_the_python() {
    let rang = make_event(Some("manager"), Action::Rang, 2, Some(Status::Http(200)), "");
    assert_eq!(line(&rang, Some("2026-11-01 04:00:00")), "2026-11-01 04:00:00 bell manager: rang 2 notes (HTTP 200)");
    let failed = make_event(Some("manager"), Action::Failed, 1, Some(Status::Label("curl exit 7".into())), "");
    assert_eq!(line(&failed, None), "bell manager: failed 1 note (curl exit 7)");
    let watching = make_event(Some("manager"), Action::Watching, 0, None, "from abc1234");
    assert_eq!(line(&watching, None), "bell manager: watching from abc1234");
    let error = make_event(None, Action::Error, 0, None, "the state file is corrupt");
    assert_eq!(line(&error, None), "bell: error the state file is corrupt");
}
