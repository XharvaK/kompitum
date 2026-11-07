//! The checks that run when a worker's turn ends: the patterns' half in the pump,
//! the test in a contained terminal of its own, and the round or the end after them.

use crate::rpc::RpcError;
use crate::runtime::{Core, now_ms};
use crate::task::checks::{
    CHECKS_PATH, ChecksFile, DiffLine, Source, TestRead, WarnRule, asked_checks, builtin_warns, forbid_hits,
    glob_match, merged, parse_checks, parse_diff, require_missing, split_entry, warn_hits,
};
use crate::task::git;
use crate::task::types::{Asked, CheckReport, CheckRun, Task, TaskState, TestRun, cut};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The round's words, exactly as the Python writes them, then the report.
pub const QA_PROMPT: &str = "The acceptance checks failed after your turn. Fix what they name, inside the same \
                             scope, run the tests again, and report in the same shape as before.\n\n";
/// How much of the report a round carries.
pub const QA_PROMPT_CHARS: usize = 6000;
/// A checked file is read only when it is no larger than this.
pub const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
/// A test with no limit of its own runs for half an hour.
pub const DEFAULT_TEST_TIMEOUT_S: u64 = 1800;

/// The checks a task runs: the file as committed at its base, so a worker can't change its
/// own checks, with what `task.start` asked laid over it, and the problems reading the file.
pub async fn spec(repo: &str, base: &str, asked: &Asked) -> Result<(ChecksFile, Vec<String>), RpcError> {
    let (file, problems) = match git::show_file(repo, base, CHECKS_PATH).await? {
        Some(bytes) => parse_checks(&String::from_utf8_lossy(&bytes)),
        None => (ChecksFile::default(), Vec::new()),
    };
    let mut both = merged(&file, &asked_checks(asked));
    if let Some(rounds) = asked.qa_rounds {
        both.qa_rounds = rounds;
    }
    Ok((both, problems))
}

/// The call's own checks, for when the committed file could not be read.
fn asked_spec(asked: &Asked) -> ChecksFile {
    let mut file = asked_checks(asked);
    if let Some(rounds) = asked.qa_rounds {
        file.qa_rounds = rounds;
    }
    file
}

/// The loop's step for a task whose turn is over: the patterns, then the test.
pub async fn pump_checking(core: &Arc<Core>, task: Task) {
    if super::stopping(core) {
        return;
    }
    match task.check.clone() {
        None => begin(core, task).await,
        Some(run) => watch(core, task, run).await,
    }
}

/// The patterns' half, then the test's terminal or a reuse of the last one.
async fn begin(core: &Arc<Core>, mut task: Task) {
    let started = now_ms();
    let mut report = CheckReport { turn: task.turns.last().map_or(0, |turn| turn.n), ..Default::default() };
    let Some(worktree) = task.worktree.clone() else {
        report.errors.push("the task has no worktree".into());
        finish(core, task, report, started).await;
        return;
    };
    let (spec, problems) = match spec(&task.repo, &task.base, &task.asked).await {
        Ok((spec, problems)) => (spec, problems),
        Err(e) => (asked_spec(&task.asked), vec![e.message]),
    };
    report.errors.extend(problems);
    for warning in &spec.warnings {
        if !task.warnings.contains(warning) {
            task.warnings.push(warning.clone());
        }
    }

    let lines = match read_diff(&worktree, &task.base).await {
        Ok(lines) => lines,
        Err(words) => {
            report.errors.push(words);
            Vec::new()
        }
    };
    let (hits, problems) = forbid_hits(&lines, &spec.forbid);
    report.forbid_checked = u32::try_from(spec.forbid.len()).unwrap_or(u32::MAX);
    report.forbid_hits = hits;
    report.errors.extend(problems);

    let globs: Vec<&str> =
        spec.require.iter().filter_map(|entry| split_entry(entry).0).filter(|glob| !glob.is_empty()).collect();
    let mut files = Vec::new();
    if !spec.require.is_empty() && !globs.is_empty() {
        let names = match working_names(&worktree).await {
            Ok(names) => names,
            Err(words) => {
                report.errors.push(words);
                Vec::new()
            }
        };
        let wanted: Vec<String> =
            names.into_iter().filter(|name| globs.iter().any(|glob| glob_match(name, glob))).collect();
        let root = PathBuf::from(&worktree);
        files = tokio::task::spawn_blocking(move || {
            wanted
                .into_iter()
                .filter_map(|name| read_text(&root.join(&name)).map(|text| (name, text)))
                .collect::<Vec<_>>()
        })
        .await
        .unwrap_or_default();
    }
    let (missing, problems) = require_missing(&files, &spec.require);
    report.require_checked = u32::try_from(spec.require.len()).unwrap_or(u32::MAX);
    report.require_missing = missing;
    report.errors.extend(problems);

    report.checks_file_changed = has_checks(&task).await && checks_file_changed(&worktree).await;

    let mut rules = builtin_warns();
    let call = asked_checks(&task.asked).warn;
    let file_count = spec.warn.len().saturating_sub(call.len());
    for (index, entry) in spec.warn.iter().enumerate() {
        let source = if index >= file_count { Source::Call } else { Source::File };
        match WarnRule::new(entry, source) {
            Ok(rule) => rules.push(rule),
            // A warn never fails a task, so its bad pattern is said once and skipped.
            Err(words) => {
                if !task.warnings.contains(&words) {
                    task.warnings.push(words);
                }
            }
        }
    }
    let diff = lines.clone();
    let root = PathBuf::from(&worktree);
    report.warn_hits = tokio::task::spawn_blocking(move || {
        let cache: RefCell<HashMap<String, Option<String>>> = RefCell::new(HashMap::new());
        warn_hits(&diff, &rules, |path| {
            cache.borrow_mut().entry(path.to_string()).or_insert_with(|| read_text(&root.join(path))).clone()
        })
    })
    .await
    .unwrap_or_default();

    let Some(cmd) = spec.test_cmd.clone() else {
        finish(core, task, report, started).await;
        return;
    };
    if let Some(test) = reused_test(&task, &worktree, &cmd, &spec.reuse_test_for).await {
        report.test = Some(test);
        finish(core, task, report, started).await;
        return;
    }
    start_test(core, task, report, cmd, spec.test_timeout_seconds, started).await;
}

/// The test's own terminal, through the runner, as a turn is started.
async fn start_test(
    core: &Arc<Core>,
    mut task: Task,
    report: CheckReport,
    cmd: String,
    timeout_s: Option<u64>,
    started: u64,
) {
    let Some(worktree) = task.worktree.clone() else {
        let mut report = report;
        report.errors.push("the task has no worktree".into());
        finish(core, task, report, started).await;
        return;
    };
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(e) => {
            let mut report = report;
            report.errors.push(format!("cannot find this lc: {e}"));
            finish(core, task, report, started).await;
            return;
        }
    };
    let folder = super::task_folder(&core.paths.state, &task.name).display().to_string();
    let mut argv = vec![
        exe.display().to_string(),
        "run-logged".into(),
        "--check".into(),
        "--dir".into(),
        folder,
        "--task".into(),
        task.name.clone(),
        "--turn".into(),
        report.turn.to_string(),
        "--worker".into(),
        "check".into(),
    ];
    if let Some(first) = task.turns.last().and_then(|turn| turn.path_first.clone()) {
        argv.push("--path-first".into());
        argv.push(first);
    }
    argv.push("--".into());
    argv.extend(crate::platform::shell_line(&cmd));
    let mut attempt = start_check(core, &task, &worktree, &argv, check_name(&task.name)).await;
    if let Err(e) = &attempt
        && e.kind == "name_taken"
    {
        // A live terminal already holds `<task>.check`: the address still finds this one.
        attempt = start_check(core, &task, &worktree, &argv, fallback_name(&task.name, task.seq)).await;
    }
    match attempt {
        Ok(address) => {
            task.check = Some(CheckRun {
                turn: report.turn,
                terminal: Some(address),
                cmd,
                started_ms: now_ms(),
                timeout_s: timeout_s.unwrap_or(DEFAULT_TEST_TIMEOUT_S),
                report,
            });
            super::save(&core.paths.state, &task);
            super::replace(core, task);
            core.changed.notify_waiters();
        }
        Err(e) => {
            let mut report = report;
            report.errors.push(format!("the test's terminal could not start: {}", e.message));
            finish(core, task, report, started).await;
        }
    }
}

/// The check's terminal, under a name that might already be held.
async fn start_check(
    core: &Arc<Core>,
    task: &Task,
    worktree: &str,
    argv: &[String],
    name: String,
) -> Result<String, RpcError> {
    let params = crate::api::RunParams {
        argv: argv.to_vec(),
        cwd: Some(worktree.to_string()),
        name: Some(name),
        labels: vec![
            crate::api::Label("task".into(), task.name.clone()),
            crate::api::Label("check".into(), "check".into()),
        ],
        contain: true,
        no_parent: true,
        no_notify: true,
        outlive_parent: true,
        env: BTreeMap::from([
            ("PYTHONDONTWRITEBYTECODE".to_string(), "1".to_string()),
            ("PYTHONIOENCODING".to_string(), "utf-8".to_string()),
        ]),
        ..Default::default()
    };
    Ok(crate::api::start_terminal(core, params, None).await?.address)
}

/// A running test: wait, cut it at its limit, or read what it left.
async fn watch(core: &Arc<Core>, task: Task, run: CheckRun) {
    let now = now_ms();
    let terminal = super::terminal_now(core, run.terminal.as_deref());
    if terminal.as_ref().is_some_and(|terminal| terminal.live) {
        if now.saturating_sub(run.started_ms) > run.timeout_s.saturating_mul(1000) {
            super::kill_terminal(core, run.terminal.as_deref()).await;
            let seconds = now.saturating_sub(run.started_ms) as f64 / 1000.0;
            let log_path = super::task_folder(&core.paths.state, &task.name).join(format!("check-{}.log", run.turn));
            let log = log_path.display().to_string();
            let read = tokio::task::spawn_blocking(move || read_log(&log_path)).await.unwrap_or_default();
            let mut report = run.report;
            report.seconds = seconds;
            report.test = Some(TestRun {
                cmd: run.cmd,
                exit_code: None,
                ok: false,
                timed_out: true,
                seconds,
                summary: read.summary,
                failed: read.failed,
                tail: read.tail,
                log: Some(log),
                ..Default::default()
            });
            finish(core, task, report, run.started_ms).await;
        }
        return;
    }
    let folder = super::task_folder(&core.paths.state, &task.name);
    let log_path = folder.join(format!("check-{}.log", run.turn));
    let log = log_path.display().to_string();
    let exit_path = folder.join(format!("check-{}.exit", run.turn));
    let (code, read) =
        tokio::task::spawn_blocking(move || (read_code(&exit_path), read_log(&log_path))).await.unwrap_or_default();
    let seconds = now_ms().saturating_sub(run.started_ms) as f64 / 1000.0;
    let Some(code) = code else {
        let mut report = run.report;
        report.seconds = seconds;
        report.errors.push("the test's end was lost".into());
        finish(core, task, report, run.started_ms).await;
        return;
    };
    let mut report = run.report;
    report.seconds = seconds;
    report.test = Some(TestRun {
        cmd: run.cmd,
        exit_code: Some(code),
        ok: code == 0,
        timed_out: false,
        seconds,
        summary: read.summary,
        failed: read.failed,
        tail: read.tail,
        log: Some(log),
        ..Default::default()
    });
    finish(core, task, report, run.started_ms).await;
}

/// The verdict, then a QA round or the end. `started` is when the check began, in ms.
async fn finish(core: &Arc<Core>, mut task: Task, mut report: CheckReport, started: u64) {
    let (spec, problems) = match spec(&task.repo, &task.base, &task.asked).await {
        Ok((spec, problems)) => (spec, problems),
        Err(e) => (asked_spec(&task.asked), vec![e.message]),
    };
    for problem in problems {
        if !report.errors.contains(&problem) {
            report.errors.push(problem);
        }
    }
    let rounds = spec.qa_rounds;
    if report.seconds == 0.0 {
        report.seconds = now_ms().saturating_sub(started) as f64 / 1000.0;
    }
    report.ok = report.test.as_ref().is_none_or(|test| test.ok)
        && report.forbid_hits.is_empty()
        && report.require_missing.is_empty()
        && !report.checks_file_changed
        && report.errors.is_empty();
    if after_checks(&report, task.qa_used, rounds) == AfterChecks::Round {
        if let Some(test) = report.test.as_mut().filter(|test| test.ok)
            && let Some(worktree) = task.worktree.as_deref()
        {
            test.fingerprint = fingerprint(worktree, &task.base).await;
        }
        let path = super::task_folder(&core.paths.state, &task.name)
            .join(format!("turn-{}.followup.md", super::next_turn(&task)));
        let text = format!("{QA_PROMPT}{}", cut(&report.body(), QA_PROMPT_CHARS));
        match crate::paths::write_atomic(&path, text.as_bytes()) {
            Ok(()) => {
                task.qa_used += 1;
                task.followup_role = Some("qa".into());
                task.state = TaskState::Queued;
                task.waiting = Some(format!("checks failed: QA round {} of {rounds}", task.qa_used));
                task.ended_ms = None;
                task.check = None;
                task.checks.push(report);
                super::save(&core.paths.state, &task);
                super::replace(core, task);
                core.changed.notify_waiters();
                return;
            }
            Err(e) => {
                report.errors.push(format!("cannot write {}: {e}", path.display()));
                report.ok = false;
            }
        }
    }
    task.state = TaskState::Done;
    task.ended_ms = Some(now_ms());
    task.check = None;
    task.checks.push(report);
    super::save(&core.paths.state, &task);
    super::write_result(&core.paths.state, &task);
    super::replace(core, task);
    core.changed.notify_waiters();
}

/// What comes after a finished report: the worker again, or the end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AfterChecks {
    Finish,
    Round,
}

/// A round only when a fixable thing failed: a worker can't fix a bad pattern or a
/// test whose terminal could not start, so errors alone are not sent back.
pub fn after_checks(report: &CheckReport, qa_used: u32, rounds: u8) -> AfterChecks {
    let fixable = report.test.as_ref().is_some_and(|test| !test.ok)
        || !report.forbid_hits.is_empty()
        || !report.require_missing.is_empty()
        || report.checks_file_changed;
    if !report.ok && qa_used < u32::from(rounds) && fixable { AfterChecks::Round } else { AfterChecks::Finish }
}

/// A turn's cost out of a transcript that sums the whole session.
pub fn turn_cost(total: f64, earlier: f64) -> f64 {
    (total - earlier).max(0.0)
}

/// The names whose content differs between two fingerprints, sorted.
pub fn round_changed(old: &BTreeMap<String, Option<String>>, new: &BTreeMap<String, Option<String>>) -> Vec<String> {
    let mut changed: Vec<String> = Vec::new();
    for (name, hash) in old {
        match new.get(name) {
            Some(other) if other == hash => {}
            _ => changed.push(name.clone()),
        }
    }
    for name in new.keys() {
        if !old.contains_key(name) {
            changed.push(name.clone());
        }
    }
    changed.sort();
    changed
}

/// The test's terminal: the task's name cut so the suffix still fits in 32.
fn check_name(task: &str) -> String {
    let cut: String = task.chars().take(26).collect();
    format!("{cut}.check")
}

/// The second name, when `<task>.check` is already a live terminal: the queue number
/// keeps it apart from every other task, and the suffix still fits in 32.
fn fallback_name(task: &str, seq: u64) -> String {
    let cut: String = task.chars().take(20).collect();
    format!("{cut}.{seq}.check")
}

/// A round that changed no file, or only files its `reuse_test_for` covers, reuses
/// the test that passed: the same run, with the reason on it.
async fn reused_test(task: &Task, worktree: &str, cmd: &str, globs: &[String]) -> Option<TestRun> {
    if task.turns.last().map(|turn| turn.role.as_str()) != Some("qa") || globs.is_empty() {
        return None;
    }
    let previous = task.checks.last()?.test.as_ref()?;
    if !previous.ok || previous.cmd != cmd {
        return None;
    }
    let before = previous.fingerprint.as_ref()?;
    let after = fingerprint(worktree, &task.base).await?;
    let changed = round_changed(before, &after);
    if changed.iter().any(|path| !globs.iter().any(|glob| glob_match(path, glob))) {
        return None;
    }
    let reason = if changed.is_empty() {
        "the round changed no file".to_string()
    } else {
        let mut names = changed.iter().take(5).cloned().collect::<Vec<_>>().join(", ");
        if changed.len() > 5 {
            names.push_str(&format!(" and {} more", changed.len() - 5));
        }
        format!("the round changed only files matching reuse_test_for ({names})")
    };
    let mut test = previous.clone();
    test.reused = Some(reason);
    Some(test)
}

/// Every changed and untracked file's hash, so a later round can see what moved.
async fn fingerprint(worktree: &str, base: &str) -> Option<BTreeMap<String, Option<String>>> {
    git::git(worktree, &["add", "-N", "."]).await.ok()?;
    let tracked = git::git(worktree, &["diff", "--name-only", "-z", "--no-renames", base]).await.ok()?;
    let untracked = git::git(worktree, &["ls-files", "--others", "--exclude-standard", "-z"]).await.ok()?;
    let mut names: Vec<String> = Vec::new();
    for output in [&tracked, &untracked] {
        for name in String::from_utf8_lossy(&output.stdout).split('\0') {
            if !name.is_empty() && !names.iter().any(|seen| seen == name) {
                names.push(name.to_string());
            }
        }
    }
    let root = PathBuf::from(worktree);
    tokio::task::spawn_blocking(move || {
        let mut map = BTreeMap::new();
        for name in names {
            let hash = match std::fs::read(root.join(&name)) {
                Ok(bytes) => {
                    let mut hasher = DefaultHasher::new();
                    bytes.hash(&mut hasher);
                    Some(format!("{:x}", hasher.finish()))
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(_) => return None,
            };
            map.insert(name, hash);
        }
        Some(map)
    })
    .await
    .ok()
    .flatten()
}

/// `git add -N .`, then the added and removed lines against the base.
async fn read_diff(worktree: &str, base: &str) -> Result<Vec<DiffLine>, String> {
    git::git(worktree, &["add", "-N", "."]).await.map_err(|e| e.message)?;
    let out = git::git(worktree, &["diff", "-U0", "--no-color", "--no-renames", base]).await.map_err(|e| e.message)?;
    Ok(parse_diff(&String::from_utf8_lossy(&out.stdout)))
}

/// `_working_names`: tracked and untracked files, NUL-separated.
async fn working_names(worktree: &str) -> Result<Vec<String>, String> {
    let out = git::git(worktree, &["ls-files", "-z", "--cached", "--others", "--exclude-standard"])
        .await
        .map_err(|e| e.message)?;
    Ok(String::from_utf8_lossy(&out.stdout).split('\0').filter(|name| !name.is_empty()).map(str::to_string).collect())
}

/// `has_checks`: the base has a checks file, or the call asked for a test, forbid or
/// require. A task that adds a fresh checks file is not changing checks that applied to it.
async fn has_checks(task: &Task) -> bool {
    let base = git::show_file(&task.repo, &task.base, CHECKS_PATH).await;
    !matches!(base, Ok(None))
        || task.asked.test.as_deref().is_some_and(|test| !test.is_empty())
        || !task.asked.forbid.is_empty()
        || !task.asked.require.is_empty()
}

/// `_checks_file_changed`: the committed checks file differs from the base, or one is untracked.
async fn checks_file_changed(worktree: &str) -> bool {
    if git::git(worktree, &["rev-parse", "--verify", "HEAD"]).await.is_ok()
        && git::git(worktree, &["diff", "--quiet", "HEAD", "--", CHECKS_PATH]).await.is_err()
    {
        return true;
    }
    matches!(
        git::git(worktree, &["ls-files", "--others", "--exclude-standard", "--", CHECKS_PATH]).await,
        Ok(out) if !String::from_utf8_lossy(&out.stdout).trim().is_empty()
    )
}

/// `_read_text`: a file's text, or None when it is unreadable, over the limit, or not UTF-8.
fn read_text(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return None;
    }
    String::from_utf8(bytes).ok()
}

/// The test's log, line by line as it was printed, each line decoded lossily.
fn read_log(path: &Path) -> TestRead {
    let mut read = TestRead::default();
    let Ok(file) = std::fs::File::open(path) else { return read };
    let mut reader = BufReader::new(file);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf) {
            Ok(0) | Err(_) => break,
            Ok(_) => read.feed(&String::from_utf8_lossy(&buf)),
        }
    }
    read
}

/// The runner's `check-{turn}.exit`, in the shape of a turn's.
fn read_code(path: &Path) -> Option<i32> {
    let bytes = std::fs::read(path).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    value.get("code")?.as_i64().and_then(|code| i32::try_from(code).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::checks::Hit;

    fn hit() -> Hit {
        Hit { pattern: "p".into(), file: "f".into(), line: 1, text: "t".into() }
    }

    #[test]
    fn a_bad_check_goes_back_only_when_it_is_fixable_and_rounds_remain() {
        let failed = TestRun { ok: false, ..Default::default() };
        let broken = CheckReport { ok: false, test: Some(failed), ..Default::default() };
        assert_eq!(after_checks(&broken, 0, 1), AfterChecks::Round);
        assert_eq!(after_checks(&broken, 0, 0), AfterChecks::Finish, "no round was asked for");
        assert_eq!(after_checks(&broken, 1, 1), AfterChecks::Finish, "the rounds ran out");
        assert_eq!(after_checks(&broken, 0, 2), AfterChecks::Round);

        let good =
            CheckReport { ok: true, test: Some(TestRun { ok: true, ..Default::default() }), ..Default::default() };
        assert_eq!(after_checks(&good, 0, 2), AfterChecks::Finish);

        let errors = CheckReport {
            ok: false,
            errors: vec!["forbid entry \"(\" is not a valid regex".into()],
            ..Default::default()
        };
        assert_eq!(after_checks(&errors, 0, 2), AfterChecks::Finish, "a worker can't fix a bad pattern");

        let forbidden = CheckReport {
            ok: false,
            test: Some(TestRun { ok: true, ..Default::default() }),
            forbid_hits: vec![hit()],
            ..Default::default()
        };
        assert_eq!(after_checks(&forbidden, 0, 1), AfterChecks::Round);
        let missing = CheckReport { ok: false, require_missing: vec!["a::b".into()], ..Default::default() };
        assert_eq!(after_checks(&missing, 0, 1), AfterChecks::Round);
        let changed = CheckReport { ok: false, checks_file_changed: true, ..Default::default() };
        assert_eq!(after_checks(&changed, 0, 1), AfterChecks::Round);
    }

    #[test]
    fn a_rounds_changed_names_are_sorted_and_none_is_none() {
        let map = |pairs: &[(&str, Option<&str>)]| {
            pairs.iter().map(|(name, hash)| (name.to_string(), hash.map(str::to_string))).collect::<BTreeMap<_, _>>()
        };
        let old = map(&[("a", Some("1")), ("b", Some("2")), ("gone", None)]);
        let new = map(&[("a", Some("1")), ("b", Some("3")), ("new", Some("9"))]);
        assert_eq!(round_changed(&old, &new), ["b", "gone", "new"]);
        assert_eq!(round_changed(&new, &new), Vec::<String>::new());
        assert_eq!(round_changed(&map(&[]), &new), ["a", "b", "new"]);
    }

    #[test]
    fn a_turn_costs_what_the_transcript_added_since_the_turns_before_it() {
        assert!((turn_cost(0.5, 0.2) - 0.3).abs() < 1e-9);
        assert_eq!(turn_cost(0.25, 0.0), 0.25);
        assert_eq!(turn_cost(0.1, 0.4), 0.0, "a total below the earlier turns never goes negative");
        assert_eq!(turn_cost(0.0, 0.0), 0.0);
    }

    #[test]
    fn a_long_task_name_still_fits_a_terminals_name() {
        let long = "a".repeat(32);
        let name = check_name(&long);
        assert_eq!(name.len(), 32);
        assert_eq!(name, format!("{}.check", "a".repeat(26)));
        assert_eq!(check_name("fix-1"), "fix-1.check");
        let fallback = fallback_name(&long, 7);
        assert_eq!(fallback, format!("{}.7.check", "a".repeat(20)));
        assert!(fallback.len() <= 32);
    }
}
