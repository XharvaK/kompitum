//! The vault's Git: pull before reading, one commit per change, push, and on a
//! rejected push pull and push again. Never a force push. Every failure comes
//! back as text to show, never swallowed. One kom at a time works in a clone:
//! a lock file in `.git` guards the pulls and the writes, and a recent pull is
//! remembered so a read does not repeat it.

use super::clock::Moment;
use std::fs::OpenOptions;
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, UNIX_EPOCH};

pub const TIMEOUT: Duration = Duration::from_secs(30);
pub const READ_TIMEOUT: Duration = Duration::from_secs(15);

/// How long a taker waits for the vault's lock, how often it looks, and how
/// old a lock may be before its holder is presumed gone.
pub const LOCK_WAIT: Duration = Duration::from_secs(20);
pub const LOCK_RETRY: Duration = Duration::from_millis(100);
/// Longer than the longest a holder works (a write's three pull-and-push rounds,
/// each git call up to 30 s), so a live holder is never taken over.
pub const LOCK_STALE: Duration = Duration::from_secs(300);

/// A read reuses a pull that finished this recently.
pub const PULLED_FRESH: Duration = Duration::from_secs(15);

const LIMITS: LockLimits = LockLimits { wait: LOCK_WAIT, retry: LOCK_RETRY, stale: LOCK_STALE };

#[derive(Clone, Copy)]
struct LockLimits {
    wait: Duration,
    retry: Duration,
    stale: Duration,
}

/// The lock while this process holds it; dropping it lets the next kom in.
pub struct VaultLock {
    path: PathBuf,
    /// What this process wrote into it, so it only ever removes its own lock.
    token: String,
}

impl Drop for VaultLock {
    fn drop(&mut self) {
        if std::fs::read_to_string(&self.path).ok().as_deref() == Some(self.token.as_str()) {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

fn lock_path(dir: &Path) -> PathBuf {
    dir.join(".git").join("kompitum-vault.lock")
}

fn pulled_path(dir: &Path) -> PathBuf {
    dir.join(".git").join("kompitum-pulled")
}

/// Whether a lock taken at `held_since` is older than `stale`.
fn lock_is_stale(held_since: i64, now: i64, stale: Duration) -> bool {
    now.saturating_sub(held_since) > stale.as_secs() as i64
}

/// Whether the taker may keep looking after waiting this long.
fn still_waiting(waited: Duration, wait: Duration) -> bool {
    waited < wait
}

/// Whether a pull that finished at `pulled_at` is still fresh enough to reuse.
fn fresh_enough(pulled_at: i64, now: i64, within: Duration) -> bool {
    now.saturating_sub(pulled_at) < within.as_secs() as i64
}

/// The epoch a lock file records, if it wrote one.
fn lock_time(text: &str) -> Option<i64> {
    text.split_whitespace().nth(1)?.parse().ok()
}

/// The time a lock file wrote, as the waiting kom shows it.
fn lock_since(text: &str) -> Option<String> {
    text.split_whitespace().nth(2).map(str::to_string)
}

/// A lock file's own age, for one whose contents cannot be read.
fn lock_mtime(path: &Path) -> Option<i64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    Some(modified.duration_since(UNIX_EPOCH).ok()?.as_secs() as i64)
}

/// Take the vault's lock, waiting for another kom or removing a stale one.
/// Hold it across `pull_under_lock`, the edit, and
/// `commit_and_push_under_lock`; dropping it lets the next kom in.
pub fn take(dir: &Path) -> Result<VaultLock, String> {
    take_with(dir, LIMITS)
}

/// Run `body` while holding the vault's lock, so its pull, edit, commit and
/// push are one piece of work to every other kom.
pub fn with_vault_lock<T>(dir: &Path, body: impl FnOnce() -> T) -> Result<T, String> {
    let _lock = take(dir)?;
    Ok(body())
}

fn take_with(dir: &Path, limits: LockLimits) -> Result<VaultLock, String> {
    let path = lock_path(dir);
    let started = Instant::now();
    loop {
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                let now = Moment::now();
                let token = format!(
                    "{} {} {}
",
                    std::process::id(),
                    now.epoch,
                    now.mark_stamp()
                );
                let _ = file.write_all(token.as_bytes());
                return Ok(VaultLock { path, token });
            }
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                let held = std::fs::read_to_string(&path).ok();
                let now = Moment::now();
                let held_since = held.as_deref().and_then(lock_time).or_else(|| lock_mtime(&path)).unwrap_or(now.epoch);
                // Take over only the stale lock just read, and only if it really went:
                // another waiter may have taken it over first.
                if lock_is_stale(held_since, now.epoch, limits.stale)
                    && std::fs::read_to_string(&path).ok() == held
                    && std::fs::remove_file(&path).is_ok()
                {
                    continue;
                }
                if !still_waiting(started.elapsed(), limits.wait) {
                    let since = held.as_deref().and_then(lock_since).unwrap_or_else(|| "an unknown time".to_string());
                    return Err(format!(
                        "Another kom is still working in the vault (since {since}). Nothing was pulled."
                    ));
                }
                std::thread::sleep(limits.retry);
            }
            Err(error) => {
                return Err(format!("The vault's lock {} could not be made: {error}", path.display()));
            }
        }
    }
}

/// Remember that a pull just succeeded, for `pull_for_reading`.
fn mark_pulled(dir: &Path) {
    let _ = std::fs::write(pulled_path(dir), format!("{}\n", Moment::now().epoch));
}

/// Whether a pull finished less than `within` ago.
fn pulled_within(dir: &Path, within: Duration) -> bool {
    let Ok(text) = std::fs::read_to_string(pulled_path(dir)) else { return false };
    let Ok(pulled_at) = text.trim().parse::<i64>() else { return false };
    fresh_enough(pulled_at, Moment::now().epoch, within)
}

#[derive(Clone, Debug, Default)]
pub struct GitResult {
    pub ok: bool,
    pub output: String,
    pub error: String,
}

pub fn run(dir: &Path, args: &[&str], timeout: Duration) -> GitResult {
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(dir)
        .args(["-c", "core.quotePath=false", "-c", "color.ui=false"])
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    crate::platform::no_console(&mut cmd);
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => return GitResult { ok: false, output: String::new(), error: tail(&e.to_string()) },
    };
    let mut stdout = child.stdout.take().expect("stdout is piped");
    let mut stderr = child.stderr.take().expect("stderr is piped");
    let out_reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes);
        bytes
    });
    let err_reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stderr.read_to_end(&mut bytes);
        bytes
    });
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = out_reader.join();
                let _ = err_reader.join();
                return GitResult { ok: false, output: String::new(), error: tail(&e.to_string()) };
            }
        }
        if start.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            let _ = out_reader.join();
            let _ = err_reader.join();
            return GitResult {
                ok: false,
                output: String::new(),
                error: format!("git {} took longer than {} s", args.join(" "), timeout.as_secs()),
            };
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let stdout = String::from_utf8_lossy(&out_reader.join().unwrap_or_default()).into_owned();
    let stderr = String::from_utf8_lossy(&err_reader.join().unwrap_or_default()).into_owned();
    if !status.success() {
        let error = if stderr.trim().is_empty() {
            tail(&format!("git {} exited {}", args.join(" "), status.code().unwrap_or(-1)))
        } else {
            tail(&stderr)
        };
        return GitResult { ok: false, output: stdout, error };
    }
    GitResult { ok: true, output: stdout, error: String::new() }
}

/// Bring the vault up to date. A vault with no remote is left as it is.
pub fn pull(dir: &Path, timeout: Duration) -> GitResult {
    match take(dir) {
        Ok(_lock) => pull_under_lock(dir, timeout),
        Err(error) => GitResult { ok: false, output: String::new(), error },
    }
}

/// The pull itself, with the lock already held. Success is remembered.
pub fn pull_under_lock(dir: &Path, timeout: Duration) -> GitResult {
    let result = pull_once(dir, timeout);
    if result.ok {
        mark_pulled(dir);
    }
    result
}

fn pull_once(dir: &Path, timeout: Duration) -> GitResult {
    let remotes = run(dir, &["remote"], timeout);
    if !remotes.ok {
        return remotes;
    }
    if remotes.output.trim().is_empty() {
        return GitResult { ok: true, output: String::new(), error: String::new() };
    }
    run(dir, &["pull", "--rebase", "--quiet"], timeout)
}

/// The line a read says when its pull failed, so an old copy never passes for
/// the vault.
pub fn behind_line(error: &str) -> String {
    let first = error.lines().next().unwrap_or("").trim();
    format!("The vault couldn't be updated, so this may be behind (git: {first}).")
}

/// Pull before a read unless a pull finished less than 15 seconds ago. The
/// line to say first when that failed, so an old copy never passes for the
/// vault; else empty.
pub fn pull_for_reading(dir: &Path) -> String {
    if pulled_within(dir, PULLED_FRESH) {
        return String::new();
    }
    let pulled = pull(dir, READ_TIMEOUT);
    if pulled.ok { String::new() } else { behind_line(&pulled.error) }
}

/// Commit the given vault paths with the message and push. Retries a rejected
/// push after a pull, up to three times. The writers hold one lock across the
/// whole piece, so only the split-take regression uses this on its own.
#[cfg(test)]
pub fn commit_and_push(dir: &Path, paths: &[String], message: &str) -> GitResult {
    match take(dir) {
        Ok(_lock) => commit_and_push_under_lock(dir, paths, message),
        Err(error) => GitResult { ok: false, output: String::new(), error },
    }
}

/// The commit and push themselves, with the lock already held.
pub fn commit_and_push_under_lock(dir: &Path, paths: &[String], message: &str) -> GitResult {
    let mut add: Vec<&str> = vec!["add", "--"];
    add.extend(paths.iter().map(String::as_str));
    let added = run(dir, &add, TIMEOUT);
    if !added.ok {
        return added;
    }
    let staged = run(dir, &["diff", "--cached", "--name-only"], TIMEOUT);
    if !staged.ok {
        return staged;
    }
    if staged.output.trim().is_empty() {
        return GitResult { ok: true, output: "Nothing changed.".into(), error: String::new() };
    }
    let mut commit: Vec<&str> = vec!["commit", "--quiet", "-m", message, "--"];
    commit.extend(paths.iter().map(String::as_str));
    let committed = run(dir, &commit, TIMEOUT);
    if !committed.ok {
        return committed;
    }
    let remotes = run(dir, &["remote"], TIMEOUT);
    if !remotes.ok {
        return remotes;
    }
    if remotes.output.trim().is_empty() {
        return GitResult { ok: true, output: "Committed (the vault has no remote).".into(), error: String::new() };
    }
    let mut last = GitResult { ok: false, output: String::new(), error: "not pushed".into() };
    for _ in 0..3 {
        last = run(dir, &["push", "--quiet"], TIMEOUT);
        if last.ok {
            return GitResult { ok: true, output: "Committed and pushed.".into(), error: String::new() };
        }
        let again = pull_under_lock(dir, TIMEOUT);
        if !again.ok {
            return GitResult {
                ok: false,
                output: String::new(),
                error: format!("Committed, but the push was refused and the pull failed: {}", again.error),
            };
        }
    }
    GitResult {
        ok: false,
        output: String::new(),
        error: format!("Committed, but the push failed three times: {}", last.error),
    }
}

/// The commit `HEAD` names, or why it does not.
pub fn head(dir: &Path) -> Result<String, String> {
    let out = run(dir, &["rev-parse", "HEAD"], TIMEOUT);
    if out.ok { Ok(out.output.trim().to_string()) } else { Err(out.error) }
}

/// Message paths added after `seen` (an ancestor of HEAD).
pub fn added_since(dir: &Path, seen: &str) -> Result<Vec<String>, String> {
    let range = format!("{seen}..HEAD");
    let out = run(dir, &["log", "--format=", "--name-status", "--diff-filter=A", "--no-renames", &range], TIMEOUT);
    if !out.ok {
        return Err(out.error);
    }
    let mut paths = Vec::new();
    for line in out.output.lines() {
        let mut fields = line.split('\t');
        let status = fields.next().unwrap_or_default();
        let path = fields.next().unwrap_or_default();
        if status == "A" && !path.is_empty() {
            paths.push(path.to_string());
        }
    }
    Ok(paths)
}

/// Commits ahead of the upstream branch. One push is tried under the vault
/// lock; what is still only here comes back with the count and the reason.
pub fn unpushed(dir: &Path) -> (usize, Option<String>) {
    match take(dir) {
        Ok(_lock) => unpushed_under_lock(dir),
        Err(error) => {
            let count = ahead_count(dir);
            if count == 0 {
                (0, None)
            } else {
                (count, Some(format!("the vault has {count} unpushed note(s): {error}")))
            }
        }
    }
}

/// The same check and push, when the caller already holds the vault lock.
pub fn unpushed_under_lock(dir: &Path) -> (usize, Option<String>) {
    let count = ahead_count(dir);
    if count == 0 {
        return (0, None);
    }
    let pushed = run(dir, &["push", "--quiet"], TIMEOUT);
    if pushed.ok {
        return (0, None);
    }
    (count, Some(format!("the vault has {count} unpushed note(s): {}", pushed.error)))
}

fn ahead_count(dir: &Path) -> usize {
    let ahead = run(dir, &["rev-list", "--count", "@{upstream}..HEAD"], TIMEOUT);
    if ahead.ok { ahead.output.trim().parse().unwrap_or(0) } else { 0 }
}

/// The paths `git status` names, one per line, with renames resolved to the new one.
pub fn status_paths(dir: &Path) -> Vec<String> {
    let out = run(dir, &["status", "--porcelain"], TIMEOUT);
    if !out.ok {
        return Vec::new();
    }
    out.output
        .lines()
        .filter_map(|line| {
            if line.len() < 4 {
                return None;
            }
            let rest = &line[3..];
            let path = rest.rsplit(" -> ").next().unwrap_or(rest);
            Some(path.trim_matches('"').to_string())
        })
        .collect()
}

fn tail(output: &str) -> String {
    let lines: Vec<&str> = output.trim().lines().collect();
    lines[lines.len().saturating_sub(3)..].join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kom-vault-git-test-{}-{name}", std::process::id()));
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

    #[test]
    fn a_rejected_push_pulls_and_pushes_again_and_nothing_changed_says_so() {
        let root = scratch("push");
        let remote = root.join("remote.git");
        let a = root.join("a");
        let b = root.join("b");
        command(&root, &["init", "-q", "--bare", "-b", "main", remote.to_str().unwrap()]);
        command(&root, &["init", "-q", "-b", "main", a.to_str().unwrap()]);
        configure(&a);
        std::fs::write(a.join("README.md"), "# Vault\n").unwrap();
        command(&a, &["add", "."]);
        command(&a, &["commit", "-q", "-m", "seed"]);
        command(&a, &["remote", "add", "origin", remote.to_str().unwrap()]);
        command(&a, &["push", "-q", "-u", "origin", "main"]);
        command(&root, &["clone", "-q", remote.to_str().unwrap(), b.to_str().unwrap()]);
        configure(&b);

        std::fs::write(b.join("b.md"), "# B\n").unwrap();
        command(&b, &["add", "."]);
        command(&b, &["commit", "-q", "-m", "b: note"]);
        command(&b, &["push", "-q"]);

        std::fs::write(a.join("a.md"), "# A\n").unwrap();
        let paths = vec!["a.md".to_string()];
        let done = commit_and_push(&a, &paths, "a: note");
        assert!(done.ok, "{}", done.error);
        assert_eq!(done.output, "Committed and pushed.");
        command(&b, &["pull", "-q", "--rebase"]);
        assert_eq!(std::fs::read_to_string(b.join("a.md")).unwrap(), "# A\n");
        assert_eq!(std::fs::read_to_string(a.join("b.md")).unwrap(), "# B\n");

        let again = commit_and_push(&a, &paths, "a: again");
        assert!(again.ok, "{}", again.error);
        assert_eq!(again.output, "Nothing changed.");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_vault_with_no_remote_is_left_alone_and_a_bad_one_is_named() {
        let root = scratch("pull");
        command(&root, &["init", "-q", "-b", "main"]);
        configure(&root);
        let quiet = pull(&root, TIMEOUT);
        assert!(quiet.ok && quiet.output.is_empty());
        assert_eq!(unpushed(&root), (0, None));
        command(&root, &["remote", "add", "origin", root.join("missing.git").to_str().unwrap()]);
        let failed = pull(&root, TIMEOUT);
        assert!(!failed.ok);
        assert!(!failed.error.is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    fn short_limits(wait: Duration) -> LockLimits {
        LockLimits { wait, retry: Duration::from_millis(10), stale: Duration::from_secs(60) }
    }

    #[test]
    fn staleness_and_freshness_are_decided_plainly() {
        assert!(lock_is_stale(1_000, 1_061, Duration::from_secs(60)));
        assert!(!lock_is_stale(1_000, 1_060, Duration::from_secs(60)));
        assert!(!lock_is_stale(1_000, 999, Duration::from_secs(60)));
        assert!(still_waiting(Duration::from_millis(99), Duration::from_millis(100)));
        assert!(!still_waiting(Duration::from_millis(100), Duration::from_millis(100)));
        assert!(fresh_enough(1_000, 1_014, PULLED_FRESH));
        assert!(!fresh_enough(1_000, 1_015, PULLED_FRESH));
    }

    #[test]
    fn a_second_taker_waits_for_the_first_and_a_stale_lock_is_taken() {
        let root = scratch("lock");
        command(&root, &["init", "-q", "-b", "main"]);
        let limits = short_limits(Duration::from_secs(2));
        let first = take_with(&root, limits).unwrap();
        assert!(lock_path(&root).exists());
        let waiter = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            drop(first);
        });
        let second = take_with(&root, limits).expect("the second taker gets it after the first lets go");
        waiter.join().unwrap();
        assert!(lock_path(&root).exists(), "the second taker holds it");
        drop(second);
        assert!(!lock_path(&root).exists(), "and lets go when it ends");

        let old = Moment::now().epoch - 61;
        std::fs::write(lock_path(&root), format!("999 {old} 2020-01-01T00:00:00+00:00\n")).unwrap();
        let taken = take_with(&root, limits).expect("a stale lock is removed and taken");
        assert!(lock_path(&root).exists());
        drop(taken);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_holder_never_removes_a_lock_another_kom_took_over() {
        let root = scratch("lock-owner");
        command(&root, &["init", "-q", "-b", "main"]);
        let mine = take_with(&root, short_limits(Duration::from_secs(1))).unwrap();
        std::fs::write(
            lock_path(&root),
            "4242 1 2020-01-01T00:00:00+00:00
",
        )
        .unwrap();
        drop(mine);
        assert!(lock_path(&root).exists(), "the other kom's lock stays");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_lock_that_never_goes_names_its_holder_and_says_nothing_was_pulled() {
        let root = scratch("lock-held");
        command(&root, &["init", "-q", "-b", "main"]);
        let since = Moment::now().mark_stamp();
        std::fs::write(lock_path(&root), format!("999 {} {since}\n", Moment::now().epoch)).unwrap();
        let error =
            take_with(&root, short_limits(Duration::from_millis(50))).err().expect("a held lock is never taken");
        assert_eq!(error, format!("Another kom is still working in the vault (since {since}). Nothing was pulled."));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn two_threads_pull_the_same_vault_at_once_and_both_succeed() {
        let root = scratch("threads");
        let remote = root.join("remote.git");
        let clone = root.join("clone");
        command(&root, &["init", "-q", "--bare", "-b", "main", remote.to_str().unwrap()]);
        command(&root, &["init", "-q", "-b", "main", clone.to_str().unwrap()]);
        configure(&clone);
        std::fs::write(clone.join("README.md"), "# Vault\n").unwrap();
        command(&clone, &["add", "."]);
        command(&clone, &["commit", "-q", "-m", "seed"]);
        command(&clone, &["remote", "add", "origin", remote.to_str().unwrap()]);
        command(&clone, &["push", "-q", "-u", "origin", "main"]);

        let one = clone.clone();
        let two = clone.clone();
        let first = std::thread::spawn(move || pull(&one, TIMEOUT));
        let second = std::thread::spawn(move || pull(&two, TIMEOUT));
        let (a, b) = (first.join().unwrap(), second.join().unwrap());
        assert!(a.ok, "{}", a.error);
        assert!(b.ok, "{}", b.error);
        assert!(pulled_within(&clone, PULLED_FRESH), "a successful pull is remembered");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_read_reuses_a_recent_pull_but_a_write_always_pulls() {
        let root = scratch("fresh");
        let remote = root.join("remote.git");
        command(&root, &["init", "-q", "--bare", "-b", "main", remote.to_str().unwrap()]);
        command(&root, &["init", "-q", "-b", "main", root.to_str().unwrap()]);
        configure(&root);
        std::fs::write(root.join("README.md"), "# Vault\n").unwrap();
        command(&root, &["add", "."]);
        command(&root, &["commit", "-q", "-m", "seed"]);
        command(&root, &["remote", "add", "origin", remote.to_str().unwrap()]);
        command(&root, &["push", "-q", "-u", "origin", "main"]);

        let pulled = pull(&root, TIMEOUT);
        assert!(pulled.ok, "{}", pulled.error);
        assert!(pulled_within(&root, PULLED_FRESH));
        command(&root, &["remote", "set-url", "origin", root.join("missing.git").to_str().unwrap()]);
        assert_eq!(pull_for_reading(&root), "", "a read within 15 seconds does not run git");
        assert!(!pull(&root, TIMEOUT).ok, "a write always pulls");

        let old = Moment::now().epoch - 16;
        std::fs::write(pulled_path(&root), format!("{old}\n")).unwrap();
        assert!(!pulled_within(&root, PULLED_FRESH));
        let line = pull_for_reading(&root);
        assert!(line.starts_with("The vault couldn't be updated"), "{line}");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_failed_read_pull_says_what_it_means_and_keeps_gits_first_line() {
        let root = scratch("wording");
        command(&root, &["init", "-q", "-b", "main"]);
        configure(&root);
        command(&root, &["remote", "add", "origin", root.join("missing.git").to_str().unwrap()]);
        let failed = pull(&root, TIMEOUT);
        assert!(!failed.ok);
        let first = failed.error.lines().next().unwrap().trim();
        let line = pull_for_reading(&root);
        assert_eq!(line, format!("The vault couldn't be updated, so this may be behind (git: {first})."));
        std::fs::remove_dir_all(root).unwrap();
    }

    fn clone_with_remote(name: &str) -> (PathBuf, PathBuf) {
        let root = scratch(name);
        let remote = root.join("remote.git");
        let clone = root.join("clone");
        command(&root, &["init", "-q", "--bare", "-b", "main", remote.to_str().unwrap()]);
        command(&root, &["init", "-q", "-b", "main", clone.to_str().unwrap()]);
        configure(&clone);
        std::fs::write(clone.join("shared.md"), "# Shared\n").unwrap();
        command(&clone, &["add", "."]);
        command(&clone, &["commit", "-q", "-m", "seed"]);
        command(&clone, &["remote", "add", "origin", remote.to_str().unwrap()]);
        command(&clone, &["push", "-q", "-u", "origin", "main"]);
        (root, clone)
    }

    fn edit_shared(root: &Path, text: &str) {
        std::fs::write(root.join("shared.md"), text).unwrap();
    }

    /// Pull, edit and commit with one lock held across all three.
    fn locked_edit(root: &Path, text: &str) -> GitResult {
        let lock = take(root).expect("the vault's lock");
        let pulled = pull_under_lock(root, TIMEOUT);
        if !pulled.ok {
            drop(lock);
            return pulled;
        }
        edit_shared(root, text);
        let result = commit_and_push_under_lock(root, &["shared.md".to_string()], "locked edit");
        drop(lock);
        result
    }

    #[test]
    fn one_lock_across_pull_edit_commit_keeps_two_writers_that_split_takes_lose() {
        use std::sync::{Arc, Barrier};

        // Split takes: A pulls and edits the tracked note without a lock, and
        // B's pull arrives before A commits. The edit is still uncommitted, so
        // B's pull collides with it.
        let (root, clone) = clone_with_remote("split-race");
        let edited = Arc::new(Barrier::new(2));
        let attempted = Arc::new(Barrier::new(2));
        let slow = clone.clone();
        let fast = clone;
        let (slow_edited, fast_edited) = (Arc::clone(&edited), Arc::clone(&edited));
        let (slow_attempted, fast_attempted) = (Arc::clone(&attempted), Arc::clone(&attempted));
        let slow = std::thread::spawn(move || {
            let pulled = pull(&slow, TIMEOUT);
            assert!(pulled.ok, "{}", pulled.error);
            edit_shared(&slow, "A\n");
            slow_edited.wait();
            slow_attempted.wait();
            commit_and_push(&slow, &["shared.md".to_string()], "a: edit")
        });
        let fast = std::thread::spawn(move || {
            fast_edited.wait();
            let pulled = pull(&fast, TIMEOUT);
            fast_attempted.wait();
            pulled
        });
        let slow = slow.join().unwrap();
        let fast = fast.join().unwrap();
        assert!(slow.ok, "A commits its edit: {}", slow.error);
        assert!(!fast.ok, "B's pull collides with A's uncommitted edit");
        assert!(!fast.error.is_empty(), "the collision is said in words");
        std::fs::remove_dir_all(root).unwrap();

        // One held lock: each writer pulls, edits, commits and pushes under
        // it. The second waits, then works on top of the first; both succeed.
        let (root, clone) = clone_with_remote("locked-race");
        let start = Arc::new(Barrier::new(2));
        let one = clone.clone();
        let two = clone.clone();
        let (one_start, two_start) = (Arc::clone(&start), Arc::clone(&start));
        let first = std::thread::spawn(move || {
            one_start.wait();
            locked_edit(&one, "A\n")
        });
        let second = std::thread::spawn(move || {
            two_start.wait();
            locked_edit(&two, "B\n")
        });
        let one = first.join().unwrap();
        let two = second.join().unwrap();
        assert!(one.ok, "the first held write succeeds: {}", one.error);
        assert!(two.ok, "the second held write waits and succeeds: {}", two.error);
        assert_eq!(command(&clone, &["rev-list", "--count", "HEAD"]).trim(), "3", "both edits are commits on the seed");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unpushed_waits_for_the_vault_lock_before_it_pushes() {
        let (root, clone) = clone_with_remote("unpushed-lock");
        std::fs::write(clone.join("extra.md"), "x\n").unwrap();
        command(&clone, &["add", "extra.md"]);
        command(&clone, &["commit", "-q", "-m", "local only"]);
        assert_eq!(command(&clone, &["rev-list", "--count", "@{upstream}..HEAD"]).trim(), "1");

        let held = take(&clone).expect("the vault's lock");
        let worker = clone.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            let result = unpushed(&worker);
            tx.send(()).unwrap();
            result
        });
        let finished_while_held = rx.recv_timeout(Duration::from_millis(400)).is_ok();
        drop(held);
        let result = thread.join().unwrap();
        assert!(!finished_while_held, "unpushed pushed without the vault lock");
        assert_eq!(result, (0, None), "{result:?}");
        std::fs::remove_dir_all(root).unwrap();
    }
}
