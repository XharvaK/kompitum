//! The vault as notes. A message is any note at
//! `<sender>/to/<recipient>/<file>.md`: it lives in the sender's folder, so no
//! agent writes into another's. It is open for its recipient until it is
//! marked answered or superseded, until the recipient answers with `re:`
//! naming it, or (for a decision or a try) until the recipient reads it.

use super::clock::{self, Moment};
use super::git;
use super::listing::{
    changes_notes, clean_path, cut_body, find_notes, message_of, message_path, open_messages, read_one, stem,
    thread_notes, truncate,
};
use super::marks::{self, Done, ReadBy};
use super::notes::{Meta, STATUSES};
use super::pen::{mark_note, only_pending, write_message};
use crate::rpc::RpcError;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// A message body is at most this many characters.
pub(super) const BODY_MAX: usize = 200_000;
/// A body shown by `read` or `thread` stops here.
pub(super) const SHOWN: usize = 4000;
const WEEK: i64 = 7;

/// Where the vault is and who is reading it.
#[derive(Clone, Debug)]
pub struct Vault {
    pub root: PathBuf,
    pub agent: String,
}

impl Vault {
    pub fn from_env(agent: Option<&str>) -> Result<Vault, RpcError> {
        let raw = std::env::var_os("KOMPITUM_VAULT")
            .map(PathBuf::from)
            .unwrap_or_else(|| home().join("Obsidian").join("vault"));
        let root = std::path::absolute(&raw)
            .map_err(|e| RpcError::new("config", format!("cannot place {}: {e}", raw.display()), ""))?;
        if !root.join(".git").exists() {
            return Err(RpcError::new(
                "config",
                format!("No Git vault at {}.", root.display()),
                "set KOMPITUM_VAULT to the vault's folder",
            ));
        }
        let agent = agent
            .map(str::to_string)
            .or_else(|| std::env::var("KOMPITUM_AGENT").ok())
            .unwrap_or_else(|| "opus".to_string());
        Ok(Vault { root, agent })
    }

    fn root_text(&self) -> String {
        self.root.display().to_string().replace('\\', "/")
    }

    /// The messages open for me, loudest first: requests, questions and notes
    /// with no kind, then the rest, newest first inside each.
    pub fn open(&self) -> Result<Report, RpcError> {
        let (unpushed, mut problems) = sync_read(&self.root);
        let index = marks::read_all(&self.root);
        problems.extend(index.problems.iter().cloned());
        let (notes, note_problems) = open_messages(&self.root, &index, &self.agent)?;
        problems.extend(note_problems);
        Ok(Report {
            vault: self.root_text(),
            agent: self.agent.clone(),
            unpushed,
            problems,
            notes,
            note: None,
            body: false,
        })
    }

    pub fn read(&self, path: &str) -> Result<Report, RpcError> {
        let path = clean_path(path);
        let mut problems = Vec::new();
        let mut index = marks::read_all(&self.root);
        let marking = message_path(&path)
            .is_some_and(|(sender, recipient)| sender != self.agent && (recipient == self.agent || recipient == "all"))
            && !index.marked_by(&path, &self.agent);
        let note = if marking {
            let note = with_lock(&self.root, "the note was not marked as read", || {
                let pulled = git::pull_under_lock(&self.root, git::TIMEOUT);
                let behind = if pulled.ok { String::new() } else { git::behind_line(&pulled.error) };
                if !behind.is_empty() {
                    problems.push(behind.clone());
                }
                let note = read_one(&self.root, &path).map_err(|e| behind_error(behind, e))?;
                index = marks::read_all(&self.root);
                if !index.marked_by(&path, &self.agent) {
                    let mark = marks::read_mark(&path, Moment::now().mark_stamp());
                    let mark_path = marks::append(&self.root, &self.agent, &mark).map_err(unavailable)?;
                    if only_pending(&self.root, &mark_path) {
                        let committed = git::commit_and_push_under_lock(
                            &self.root,
                            &[mark_path],
                            &format!("{}: read {}", self.agent, stem(&path)),
                        );
                        if !committed.ok {
                            problems.push(committed.error);
                        }
                    }
                }
                index = marks::read_all(&self.root);
                Ok(note)
            })?;
            problems.extend(index.problems.iter().cloned());
            note
        } else {
            let behind = git::pull_for_reading(&self.root);
            if !behind.is_empty() {
                problems.push(behind.clone());
            }
            let note = match read_one(&self.root, &path) {
                Ok(note) => note,
                Err(e) => return Err(behind_error(behind, e)),
            };
            index = marks::read_all(&self.root);
            problems.extend(index.problems.iter().cloned());
            note
        };
        let (unpushed, push_problem) = git::unpushed(&self.root);
        if let Some(problem) = push_problem {
            problems.push(problem);
        }
        let message = message_of(&note, &index, &self.agent);
        for problem in &message.problems {
            problems.push(format!("{}: {problem}", message.path));
        }
        Ok(Report {
            vault: self.root_text(),
            agent: self.agent.clone(),
            unpushed,
            problems,
            notes: Vec::new(),
            note: Some(message),
            body: true,
        })
    }

    pub fn find(&self, query: Query) -> Result<Report, RpcError> {
        let (unpushed, mut problems) = sync_read(&self.root);
        let index = marks::read_all(&self.root);
        problems.extend(index.problems.iter().cloned());
        let mut notes = find_notes(&self.root, &index, &self.agent, &query)?;
        truncate(&mut notes, query.limit);
        for note in &notes {
            for problem in &note.problems {
                problems.push(format!("{}: {problem}", note.path));
            }
        }
        Ok(Report {
            vault: self.root_text(),
            agent: self.agent.clone(),
            unpushed,
            problems,
            notes,
            note: None,
            body: false,
        })
    }

    /// Every message between me and another agent, both ways, oldest first.
    pub fn thread(&self, other: &str, limit: usize) -> Result<Report, RpcError> {
        let (unpushed, mut problems) = sync_read(&self.root);
        let index = marks::read_all(&self.root);
        problems.extend(index.problems.iter().cloned());
        let (mut notes, note_problems) = thread_notes(&self.root, &index, &self.agent, other)?;
        problems.extend(note_problems);
        if notes.len() > limit {
            notes = notes.split_off(notes.len() - limit);
        }
        Ok(Report {
            vault: self.root_text(),
            agent: self.agent.clone(),
            unpushed,
            problems,
            notes,
            note: None,
            body: true,
        })
    }

    /// The messages added since the last commit this reader saw.
    pub fn changes(&self) -> Result<Report, RpcError> {
        let (unpushed, mut problems) = sync_read(&self.root);
        let index = marks::read_all(&self.root);
        problems.extend(index.problems.iter().cloned());
        let (notes, change_problems) = changes_notes(&self.root, &index, &self.agent)?;
        problems.extend(change_problems);
        Ok(Report {
            vault: self.root_text(),
            agent: self.agent.clone(),
            unpushed,
            problems,
            notes,
            note: None,
            body: false,
        })
    }

    pub fn post(&self, post: Post) -> Result<Written, RpcError> {
        with_lock(&self.root, "nothing was written", || {
            let pulled = git::pull_under_lock(&self.root, git::TIMEOUT);
            if !pulled.ok {
                return Err(unavailable(format!(
                    "The vault could not be pulled, so nothing was written: {}",
                    pulled.error
                )));
            }
            let path = write_message(self, &post, &Moment::now())?;
            let mut paths = vec![path.clone()];
            let marks_path = format!("{}/reads.jsonl", self.agent);
            if self.root.join(&marks_path).exists() {
                paths.push(marks_path);
            }
            let result = git::commit_and_push_under_lock(
                &self.root,
                &paths,
                &format!("{}: {} {}", self.agent, post.kind, stem(&path)),
            );
            Ok(written(&self.root, path, result, "Wrote"))
        })
    }

    pub fn mark(&self, path: &str, status: &str) -> Result<Written, RpcError> {
        let path = clean_path(path);
        if !path.starts_with(&format!("{}/", self.agent)) || path.contains("..") {
            return Err(RpcError::new(
                "refused",
                format!("{path} is not in {}/, so it is not yours to mark.", self.agent),
                "",
            ));
        }
        if path.starts_with(&format!("{}/memory/", self.agent)) {
            return Err(RpcError::new(
                "refused",
                format!("{path} is a memory: change it with `kom vault remember --replace` or `kom vault forget`."),
                "",
            ));
        }
        if !STATUSES.contains(&status) {
            return Err(RpcError::new("usage", format!("status is one of: {}.", STATUSES.join(", ")), ""));
        }
        with_lock(&self.root, "nothing was changed", || {
            let pulled = git::pull_under_lock(&self.root, git::TIMEOUT);
            if !pulled.ok {
                return Err(unavailable(format!(
                    "The vault could not be pulled, so nothing was changed: {}",
                    pulled.error
                )));
            }
            mark_note(&self.root, &path, status)?;
            let mut paths = vec![path.clone()];
            let marks_path = format!("{}/reads.jsonl", self.agent);
            if self.root.join(&marks_path).exists() {
                paths.push(marks_path);
            }
            let result = git::commit_and_push_under_lock(
                &self.root,
                &paths,
                &format!("{}: mark {status} {}", self.agent, stem(&path)),
            );
            Ok(written(&self.root, path, result, "Marked"))
        })
    }

    pub fn done(&self, path: &str, why: Option<&str>) -> Result<Written, RpcError> {
        let path = clean_path(path);
        with_lock(&self.root, "nothing was changed", || {
            let pulled = git::pull_under_lock(&self.root, git::TIMEOUT);
            if !pulled.ok {
                return Err(unavailable(format!(
                    "The vault could not be pulled, so nothing was changed: {}",
                    pulled.error
                )));
            }
            read_one(&self.root, &path)?;
            let mark = marks::done_mark(&path, Moment::now().mark_stamp(), why.map(str::to_string));
            let mark_path = marks::append(&self.root, &self.agent, &mark).map_err(unavailable)?;
            let result = git::commit_and_push_under_lock(
                &self.root,
                &[mark_path],
                &format!("{}: done {}", self.agent, stem(&path)),
            );
            Ok(written(&self.root, path, result, "Closed"))
        })
    }
}

/// What a list command answers: the vault, the reader, what is still only here,
/// what went wrong, and the notes.
#[derive(Clone, Debug)]
pub struct Report {
    pub vault: String,
    pub agent: String,
    pub unpushed: usize,
    pub problems: Vec<String>,
    pub notes: Vec<Message>,
    pub note: Option<Message>,
    pub body: bool,
}

impl Report {
    pub fn json(&self) -> Value {
        let mut report = json!({
            "vault": self.vault,
            "agent": self.agent,
            "unpushed": self.unpushed,
            "problems": self.problems,
        });
        match &self.note {
            Some(note) => report["note"] = note.json(true),
            None => {
                report["notes"] = Value::Array(self.notes.iter().map(|note| note.json(self.body)).collect());
            }
        }
        report
    }
}

/// A message, with the marks other agents left on it.
#[derive(Clone, Debug)]
pub struct Message {
    pub path: String,
    pub meta: Meta,
    pub body: String,
    pub title: String,
    pub sender: String,
    pub recipient: String,
    pub status: String,
    pub problems: Vec<String>,
    pub read_by: Vec<ReadBy>,
    pub done: Option<Done>,
    pub snippet: Option<String>,
}

impl Message {
    pub fn kind(&self) -> Option<String> {
        self.meta.text("kind")
    }

    pub fn date(&self) -> Option<String> {
        self.meta.text("date")
    }

    pub fn project(&self) -> Option<String> {
        self.meta.text("project")
    }

    pub fn re(&self) -> Vec<String> {
        self.meta.list("re")
    }

    /// The one line `open` and `changes` write, as the Python's `line_of`.
    pub fn line(&self) -> String {
        let kind = self.kind().filter(|kind| !kind.is_empty()).unwrap_or_else(|| "note".to_string());
        let problems =
            if self.problems.is_empty() { String::new() } else { format!(" [{}]", self.problems.join("; ")) };
        let mut line =
            format!("{} → {} · {kind} · {} ({}){problems}", self.sender, self.recipient, self.title, self.path);
        if let Some(mark) = self.mark() {
            line.push_str(&format!(" · {mark}"));
        }
        line
    }

    /// `read 09:10`, or `done 09:40: why`, as a person writes it.
    pub fn mark(&self) -> Option<String> {
        if let Some(done) = &self.done {
            let mut mark = format!("done {}", short_time(&done.at));
            if let Some(why) = done.why.as_ref().filter(|why| !why.is_empty()) {
                mark.push_str(": ");
                mark.push_str(why);
            }
            return Some(mark);
        }
        self.read_by.iter().max_by(|a, b| a.at.cmp(&b.at)).map(|read| format!("read {}", short_time(&read.at)))
    }

    pub fn json(&self, body: bool) -> Value {
        let mut note = json!({
            "path": self.path,
            "from": self.sender,
            "to": self.recipient,
            "kind": self.kind(),
            "status": self.status,
            "title": self.title,
            "date": self.date(),
            "project": self.project(),
            "re": self.re(),
            "read_by": self.read_by,
            "done": self.done,
        });
        if body {
            let (text, cut) = cut_body(&self.body);
            note["body"] = text.into();
            note["cut"] = cut.into();
        }
        note
    }

    /// Newest first, with what asks for an answer before what does not.
    pub fn loud(&self) -> bool {
        matches!(self.kind().as_deref(), None | Some("") | Some("request") | Some("question"))
    }

    /// The stamp used to order it: the note's date, else the one in its file name.
    pub fn stamp(&self) -> Option<String> {
        if let Some(date) = self.date().filter(|date| !date.is_empty()) {
            return Some(date);
        }
        let name = self.path.rsplit('/').next()?;
        let stamp = name.get(..16)?;
        let (date, time) = stamp.split_once('_')?;
        Some(format!("{date}T{}:{}", time.get(..2)?, time.get(2..4)?))
    }
}

/// What a write answers: where it went, whether it got out, and the sentence a
/// person reads.
#[derive(Clone, Debug)]
pub struct Written {
    pub path: String,
    pub pushed: bool,
    pub said: String,
}

impl Written {
    pub fn json(&self) -> Value {
        json!({"path": self.path, "pushed": self.pushed, "said": self.said})
    }
}

#[derive(Clone, Debug, Default)]
pub struct Query {
    pub words: String,
    pub project: Option<String>,
    pub kind: Option<String>,
    pub status: Option<String>,
    pub from: Option<String>,
    pub limit: usize,
}

#[derive(Clone, Debug, Default)]
pub struct Post {
    pub to: String,
    pub kind: String,
    pub title: String,
    pub body: String,
    pub project: Option<String>,
    pub re: Vec<String>,
}

fn sync_read(root: &Path) -> (usize, Vec<String>) {
    let mut problems = Vec::new();
    let behind = git::pull_for_reading(root);
    if !behind.is_empty() {
        problems.push(behind);
    }
    let (unpushed, push_problem) = git::unpushed(root);
    if let Some(problem) = push_problem {
        problems.push(problem);
    }
    (unpushed, problems)
}

/// Run a vault write under one lock, from before its pull to after its push.
pub(super) fn with_lock<T>(
    root: &Path,
    nothing: &str,
    body: impl FnOnce() -> Result<T, RpcError>,
) -> Result<T, RpcError> {
    git::with_vault_lock(root, body)
        .map_err(|error| unavailable(format!("The vault could not be pulled, so {nothing}: {error}")))?
}

pub(super) fn written(root: &Path, path: String, result: git::GitResult, verb: &str) -> Written {
    let (unpushed, push_problem) = git::unpushed_under_lock(root);
    let mut said = if result.ok {
        format!("{verb} {path}. {}", result.output)
    } else {
        format!("{verb} {path}, but: {}", result.error)
    };
    if let Some(problem) = push_problem {
        said.push(' ');
        said.push_str(&problem);
    }
    Written { path, pushed: result.ok && unpushed == 0, said }
}

pub(super) fn unavailable(message: String) -> RpcError {
    RpcError::new("unavailable", message, "")
}

fn behind_error(behind: String, error: RpcError) -> RpcError {
    if behind.is_empty() {
        error
    } else {
        RpcError::new(&error.kind, format!("{behind}\n{}", error.message), error.hint)
    }
}

fn home() -> PathBuf {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_default()
}

/// `09:10` from an ISO stamp; the whole stamp when it is not one.
fn short_time(at: &str) -> &str {
    at.get(11..16).filter(|time| time.as_bytes().get(2) == Some(&b':')).unwrap_or(at)
}

/// Whether a note's stamp is more than a week old, for folding the open list.
pub fn older_than_a_week(note: &Message) -> bool {
    clock::older_than(note.stamp().as_deref(), WEEK)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kom-vault-read-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn command(dir: &Path, args: &[&str]) -> String {
        let mut cmd = Command::new("git");
        cmd.arg("-C").arg(dir).args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        crate::platform::no_console(&mut cmd);
        let out = cmd.output().unwrap();
        assert!(out.status.success(), "git {args:?} in {}: {}", dir.display(), String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    fn configure(dir: &Path) {
        for (key, value) in [
            ("user.name", "Test"),
            ("user.email", "test@example.org"),
            ("commit.gpgsign", "false"),
            ("core.autocrlf", "false"),
        ] {
            command(dir, &["config", key, value]);
        }
    }

    fn vault_with_a_note(name: &str) -> (PathBuf, PathBuf, String) {
        let root = scratch(name);
        let remote = root.join("remote.git");
        let clone = root.join("clone");
        command(&root, &["init", "-q", "--bare", "-b", "main", remote.to_str().unwrap()]);
        command(&root, &["init", "-q", "-b", "main", clone.to_str().unwrap()]);
        configure(&clone);
        let note = clone.join("manager").join("to").join("opus").join("a.md");
        std::fs::create_dir_all(note.parent().unwrap()).unwrap();
        std::fs::write(
            &note,
            "---\nfrom: manager\nto: [opus]\nkind: request\ndate: 2026-11-09T09:00+03:00\ntitle: A\n---\n\n# A\n\nBody.\n",
        )
        .unwrap();
        command(&clone, &["add", "."]);
        command(&clone, &["commit", "-q", "-m", "seed"]);
        command(&clone, &["remote", "add", "origin", remote.to_str().unwrap()]);
        command(&clone, &["push", "-q", "-u", "origin", "main"]);
        (root, clone, "manager/to/opus/a.md".to_string())
    }

    #[test]
    fn a_read_marks_under_the_lock_so_a_post_pull_never_meets_an_unstaged_mark() {
        let (root, clone, path) = vault_with_a_note("read-lock");
        let vault = Vault { root: clone.clone(), agent: "opus".into() };

        // A pull just finished, so the read's own pull takes no lock and does
        // not wait on the lock held here.
        let pulled = git::pull(&clone, git::TIMEOUT);
        assert!(pulled.ok, "{}", pulled.error);

        let held = git::take(&clone).expect("the vault's lock");
        let read_path = path.clone();
        let reader = std::thread::spawn(move || vault.read(&read_path));

        // The old read appended its mark before taking the lock, so it appeared
        // while the lock was held; the fixed read writes it only under the lock.
        let mark = clone.join("opus").join("reads.jsonl");
        let deadline = Instant::now() + Duration::from_secs(1);
        while !mark.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!mark.exists(), "the read wrote its mark without holding the vault's lock");

        // This is what a post does first, under the same lock. An unstaged mark
        // makes git refuse the pull with "cannot pull with rebase: You have
        // unstaged changes."
        let writers_pull = git::pull_under_lock(&clone, git::TIMEOUT);
        assert!(writers_pull.ok, "a post's pull met the read's unstaged mark: {}", writers_pull.error);

        drop(held);
        let report = reader.join().unwrap().expect("the read still returns the note");
        assert!(
            report.problems.iter().all(|problem| !problem.to_lowercase().contains("unstaged")),
            "{:?}",
            report.problems
        );
        let note = report.note.expect("the note");
        assert_eq!(note.path, path);
        assert_eq!(note.read_by[0].agent, "opus");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn concurrent_reads_and_a_post_wait_and_all_succeed_with_one_mark() {
        use std::sync::{Arc, Barrier};

        let (root, clone, path) = vault_with_a_note("read-post");
        let start = Arc::new(Barrier::new(3));
        let one = Vault { root: clone.clone(), agent: "opus".into() };
        let two = one.clone();
        let poster = one.clone();
        let (one_start, two_start, post_start) = (Arc::clone(&start), Arc::clone(&start), Arc::clone(&start));
        let (one_path, two_path) = (path.clone(), path.clone());
        let first = std::thread::spawn(move || {
            one_start.wait();
            one.read(&one_path)
        });
        let second = std::thread::spawn(move || {
            two_start.wait();
            two.read(&two_path)
        });
        let posted = std::thread::spawn(move || {
            post_start.wait();
            poster.post(Post {
                to: "manager".into(),
                kind: "report".into(),
                title: "Back".into(),
                body: "Done.".into(),
                ..Post::default()
            })
        });

        for report in [first.join().unwrap(), second.join().unwrap()] {
            let report = report.expect("a concurrent read succeeds");
            assert!(
                report.problems.iter().all(|problem| !problem.to_lowercase().contains("unstaged")),
                "{:?}",
                report.problems
            );
            assert_eq!(report.note.expect("the note").path, path);
        }
        let written = posted.join().unwrap().expect("the post succeeds");
        assert!(written.pushed, "{}", written.said);
        assert!(!written.said.to_lowercase().contains("unstaged"), "{}", written.said);
        let marks = std::fs::read_to_string(clone.join("opus").join("reads.jsonl")).unwrap();
        assert_eq!(marks.lines().filter(|line| line.contains("manager/to/opus/a.md")).count(), 1);
        assert!(git::status_paths(&clone).is_empty(), "the vault is left clean: {:?}", git::status_paths(&clone));
        std::fs::remove_dir_all(root).unwrap();
    }
}
