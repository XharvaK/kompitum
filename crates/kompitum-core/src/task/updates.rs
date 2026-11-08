//! The CLIs the core keeps up to date: OpenCode and Grok Build. Claude Code and
//! Command Code update themselves. Once a day, when no task or process is using
//! one, the core runs its own update and records what happened in
//! `<state>/updates.json`.

use crate::config::Config;
use crate::runtime::{Core, now_ms};
use crate::task::types::TaskState;
use crate::task::worker::{self, Cli};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;
use std::time::Instant;

/// Nothing runs in the first ten minutes after the core starts.
pub const START_GRACE_MS: u64 = 10 * 60 * 1000;
/// How often a CLI is checked when it is free.
pub const DAY_MS: u64 = 24 * 60 * 60 * 1000;
/// How long a busy CLI waits before the core asks again.
pub const BUSY_RETRY_MS: u64 = 60 * 60 * 1000;
const VERSION_LIMIT: Duration = Duration::from_secs(60);
const UPDATE_LIMIT: Duration = Duration::from_secs(15 * 60);
/// Only one update at a time.
static RUNNING: AtomicBool = AtomicBool::new(false);
/// A look reads the config, PATH and the record, so the loop's two-second tick
/// looks once a minute.
const LOOK_EVERY_MS: u64 = 60 * 1000;
static LAST_LOOK: AtomicU64 = AtomicU64::new(0);

/// One CLI the core updates. Its program is found the worker loop's way.
pub struct Spec {
    pub key: &'static str,
    pub label: &'static str,
    pub program: &'static str,
    pub version: &'static str,
    pub update: &'static str,
    pub processes: &'static [&'static str],
    /// The worker CLI it is, when the loop can run it.
    pub cli: Option<Cli>,
}

pub const SPECS: &[Spec] = &[
    Spec {
        key: "opencode",
        label: "OpenCode",
        program: "opencode",
        version: "--version",
        update: "upgrade",
        processes: &["opencode"],
        cli: Some(Cli::Opencode),
    },
    Spec {
        key: "grok",
        label: "Grok Build",
        program: "grok",
        version: "--version",
        update: "update",
        processes: &["grok"],
        cli: None,
    },
];

/// What the core knows of one CLI's update.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Entry {
    /// When the last try began; 0 when it was never tried.
    pub tried_ms: u64,
    pub before: Option<String>,
    pub after: Option<String>,
    /// When `after` differed from `before`; 0 when it did not.
    pub updated_ms: u64,
    pub error: Option<String>,
    /// When a busy CLI was put off; an hour passes before asking again.
    pub busy_ms: u64,
}

/// Whether a CLI is in use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Busy {
    No,
    Yes(String),
}

/// Whether it is time to try an update.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Due {
    No,
    Now,
    Busy,
}

/// The deciding: the start grace, a day between tries, and an hour after a busy one.
pub fn due(entry: Option<&Entry>, started_ms: u64, now_ms: u64, busy: Busy) -> Due {
    if now_ms.saturating_sub(started_ms) < START_GRACE_MS {
        return Due::No;
    }
    // A busy skip waits its hour, even when the day is already up.
    if entry.is_some_and(|entry| entry.busy_ms > 0 && now_ms.saturating_sub(entry.busy_ms) < BUSY_RETRY_MS) {
        return Due::No;
    }
    let day_due = entry.is_none_or(|entry| now_ms.saturating_sub(entry.tried_ms) >= DAY_MS);
    let busy_due =
        entry.is_some_and(|entry| entry.busy_ms > 0 && now_ms.saturating_sub(entry.busy_ms) >= BUSY_RETRY_MS);
    if !day_due && !busy_due {
        return Due::No;
    }
    match busy {
        Busy::No => Due::Now,
        Busy::Yes(_) => Due::Busy,
    }
}

/// A process name matches a CLI when it is the same name without case and `.exe`.
pub fn name_matches(process: &str, name: &str) -> bool {
    let lower = process.to_ascii_lowercase();
    let stem = lower.strip_suffix(".exe").unwrap_or(&lower);
    stem.eq_ignore_ascii_case(name)
}

/// The program and argv for each CLI installed here, in table order.
pub struct Installed {
    pub spec: &'static Spec,
    pub argv: Vec<String>,
}

/// The CLIs this machine has. A program that isn't installed is skipped.
pub fn installed(config: &Config) -> Vec<Installed> {
    let mut found = Vec::new();
    for spec in SPECS {
        let argv = if spec.key == "opencode" {
            worker::opencode_prefix(config).ok()
        } else {
            worker::on_path(&[worker::program(spec.program)]).map(|path| vec![path.display().to_string()])
        };
        if let Some(argv) = argv {
            found.push(Installed { spec, argv });
        }
    }
    found
}

/// Whether a CLI is in use: a running task's worker, or a process with its name.
/// A list that can't be read counts as busy, since an update under a running CLI
/// could break it.
pub fn busy_now(core: &Core, item: &Installed, names: Option<&[String]>) -> Busy {
    if let Some(cli) = item.spec.cli {
        let workers = crate::task::workers::all(&core.tasks.workers());
        if core.tasks.all().iter().any(|task| {
            task.state == TaskState::Running
                && task.turns.last().is_some_and(|turn| turn.cli_or_lookup(&workers) == cli)
        }) {
            return Busy::Yes("running now".into());
        }
    }
    let Some(names) = names else {
        return Busy::Yes("can't tell whether it is running".into());
    };
    if names.iter().any(|process| item.spec.processes.iter().any(|name| name_matches(process, name))) {
        return Busy::Yes("running now".into());
    }
    Busy::No
}

/// The record file, beside the tasks' folders.
pub fn path(state: &Path) -> std::path::PathBuf {
    state.join("updates.json")
}

pub fn load(state: &Path) -> BTreeMap<String, Entry> {
    let path = path(state);
    match std::fs::read(&path) {
        Ok(bytes) => match serde_json::from_slice(&bytes) {
            Ok(entries) => entries,
            Err(e) => {
                eprintln!("core: updates.json: cannot read it: {e}");
                BTreeMap::new()
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
        Err(e) => {
            eprintln!("core: updates.json: cannot read it: {e}");
            BTreeMap::new()
        }
    }
}

pub fn save(state: &Path, entries: &BTreeMap<String, Entry>) -> std::io::Result<()> {
    let bytes = serde_json::to_vec_pretty(entries).map_err(std::io::Error::other)?;
    std::fs::create_dir_all(state)?;
    crate::paths::write_atomic(&path(state), &bytes)
}

/// The pure words of one CLI's line in `kom workers`.
#[allow(clippy::too_many_arguments)]
pub fn line(
    label: &str,
    version: Option<&str>,
    before: Option<&str>,
    tried_ms: u64,
    updated_ms: u64,
    error: Option<&str>,
    waiting: Option<&str>,
    now_ms: u64,
    offset_min: i32,
) -> String {
    let name = match version.filter(|version| !version.is_empty()) {
        Some(version) => format!("{label} {version}"),
        None => label.to_string(),
    };
    if let Some(reason) = waiting {
        return format!("{name} · waiting: {reason}");
    }
    let when = day_words(if updated_ms > 0 { updated_ms } else { tried_ms }, now_ms, offset_min);
    if updated_ms > 0 {
        let changed = match before.filter(|before| Some(*before) != version) {
            Some(before) => format!("{name} · updated {when} (was {before})"),
            None => format!("{name} · updated {when}"),
        };
        return match error {
            Some(error) => format!("{changed}: {error}"),
            None => changed,
        };
    }
    if let Some(error) = error {
        return format!("{name} · update failed: {error}");
    }
    if tried_ms > 0 {
        return format!("{name} · up to date, checked {when}");
    }
    format!("{name} · not checked yet")
}

const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// A day as `today`, a weekday within a week, else a date.
fn day_words(ms: u64, now_ms: u64, offset_min: i32) -> String {
    let at = crate::task::clock::civil(ms, offset_min);
    let now = crate::task::clock::civil(now_ms, offset_min);
    if (at.year, at.month, at.day) == (now.year, now.month, now.day) {
        return "today".into();
    }
    let midnight = |c: crate::task::clock::Civil| {
        crate::task::clock::to_ms(&crate::task::clock::Civil { hour: 0, minute: 0, second: 0, ..c }, offset_min)
    };
    let days = (midnight(at) as i64 - midnight(now) as i64) / 86_400_000;
    if days.unsigned_abs() < 7 {
        return WEEKDAYS[at.weekday as usize % 7].to_string();
    }
    let year = if at.year != now.year { format!(" {}", at.year) } else { String::new() };
    format!("{} {}{}", at.day, MONTHS[(at.month - 1) as usize % 12], year)
}

/// One CLI as `kom workers` shows it.
#[derive(Serialize, JsonSchema, Clone, Debug, Default, PartialEq)]
pub struct CliUpdate {
    pub key: String,
    pub label: String,
    pub version: Option<String>,
    /// What it was before the last update that changed it.
    pub before: Option<String>,
    pub tried_ms: u64,
    pub updated_ms: u64,
    pub error: Option<String>,
    /// Why it is busy now, in words.
    pub waiting: Option<String>,
}

/// The installed CLIs and what the record says of each.
pub fn views(core: &Core, config: &Config) -> Vec<CliUpdate> {
    let entries = load(&core.paths.state);
    let names = crate::platform::process_names();
    let now = now_ms();
    let started_ms = now.saturating_sub(core.started.elapsed().as_millis() as u64);
    installed(config)
        .into_iter()
        .map(|item| {
            let entry = entries.get(item.spec.key);
            let busy = busy_now(core, &item, names.as_deref());
            // A result already on record is what the line says; a CLI that is due
            // and busy, or whose busy marker is newer than its last try, waits now.
            let due_now = due(entry, started_ms, now, Busy::No) != Due::No;
            let waiting = match busy {
                Busy::Yes(reason)
                    if due_now || entry.is_some_and(|entry| entry.busy_ms > 0 && entry.busy_ms >= entry.tried_ms) =>
                {
                    Some(reason)
                }
                _ => None,
            };
            CliUpdate {
                key: item.spec.key.into(),
                label: item.spec.label.into(),
                version: entry.and_then(|entry| entry.after.clone().or_else(|| entry.before.clone())),
                before: entry.and_then(|entry| entry.before.clone()),
                tried_ms: entry.map_or(0, |entry| entry.tried_ms),
                updated_ms: entry.map_or(0, |entry| entry.updated_ms),
                error: entry.and_then(|entry| entry.error.clone()),
                waiting,
            }
        })
        .collect()
}

/// The core's own check: at most one update at a time, and never on the pump.
pub async fn when_due(core: &std::sync::Arc<Core>) {
    if RUNNING.load(Ordering::SeqCst) {
        return;
    }
    let now = now_ms();
    let started_ms = now.saturating_sub(core.started.elapsed().as_millis() as u64);
    if now.saturating_sub(started_ms) < START_GRACE_MS
        || now.saturating_sub(LAST_LOOK.load(Ordering::SeqCst)) < LOOK_EVERY_MS
    {
        return;
    }
    LAST_LOOK.store(now, Ordering::SeqCst);
    let config = crate::config::load(&core.paths.config).unwrap_or_default();
    let installed = installed(&config);
    if installed.is_empty() {
        return;
    }
    let mut entries = load(&core.paths.state);
    let mut changed = false;
    let mut chosen: Option<Installed> = None;
    let mut names: Option<Option<Vec<String>>> = None;
    for item in installed {
        if due(entries.get(item.spec.key), started_ms, now, Busy::No) == Due::No {
            continue;
        }
        let processes = names.get_or_insert_with(crate::platform::process_names);
        match due(entries.get(item.spec.key), started_ms, now, busy_now(core, &item, processes.as_deref())) {
            Due::No => {}
            Due::Now => {
                chosen = Some(item);
                break;
            }
            Due::Busy => {
                let entry = entries.entry(item.spec.key.into()).or_default();
                if entry.busy_ms != now {
                    entry.busy_ms = now;
                    changed = true;
                }
            }
        }
    }
    if changed && let Err(e) = save(&core.paths.state, &entries) {
        eprintln!("core: updates.json: cannot save it: {e}");
    }
    let Some(item) = chosen else { return };
    if RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    let state = core.paths.state.clone();
    let spec = item.spec;
    let argv = item.argv;
    tokio::spawn(async move {
        let _ = tokio::task::spawn_blocking(move || run_update(spec, &argv, &state)).await;
        RUNNING.store(false, Ordering::SeqCst);
    });
}

/// One version, one update, one version, all headless and each under a limit.
fn run_update(spec: &Spec, argv: &[String], state: &Path) {
    let started = now_ms();
    let mut entries = load(state);
    let previous = entries.get(spec.key).cloned().unwrap_or_default();
    let mut entry = Entry { tried_ms: started, ..Default::default() };
    let mut error = None;

    match run(argv, &[spec.version], VERSION_LIMIT) {
        Err(words) => {
            error = Some(words);
            entry.before = previous.after.clone().or_else(|| previous.before.clone());
        }
        Ok(output) => match version_of(&output) {
            Err(words) => {
                error = Some(words);
                entry.before = previous.after.clone().or_else(|| previous.before.clone());
            }
            Ok(before) => {
                entry.before = Some(before.clone());
                if let Err(words) = run(argv, &[spec.update], UPDATE_LIMIT) {
                    error = Some(words);
                }
                match run(argv, &[spec.version], VERSION_LIMIT).and_then(|output| version_of(&output)) {
                    Ok(after) => {
                        entry.after = Some(after.clone());
                        if after != before {
                            entry.updated_ms = now_ms();
                        }
                        if major_changed(&before, &after) {
                            error = Some(match error {
                                Some(words) => format!("{words}; major version changed: check the workers"),
                                None => "major version changed: check the workers".into(),
                            });
                        }
                    }
                    Err(words) => {
                        if error.is_none() {
                            error = Some(words);
                        }
                    }
                }
            }
        },
    }
    entry.error = error.clone();
    entries.insert(spec.key.into(), entry);
    if let Err(e) = save(state, &entries) {
        eprintln!("core: updates.json: cannot save it: {e}");
    }
    if let Some(error) = error {
        eprintln!("core: update {}: {error}", spec.label);
    }
}

/// A command that ended but left a child holding its pipes must not hold us.
fn join_reader(reader: std::thread::JoinHandle<Vec<u8>>) -> Vec<u8> {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !reader.is_finished() {
        if Instant::now() >= deadline {
            return Vec::new();
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    reader.join().unwrap_or_default()
}

fn version_of(output: &str) -> Result<String, String> {
    first_line(output).ok_or_else(|| "it printed no version".into())
}

fn major_changed(before: &str, after: &str) -> bool {
    match (worker::first_version(before), worker::first_version(after)) {
        (Some((before, _, _)), Some((after, _, _))) => before != after,
        _ => false,
    }
}

fn first_line(text: &str) -> Option<String> {
    text.lines().map(str::trim).find(|line| !line.is_empty()).map(|line| crate::task::types::cut(line, 200))
}

/// Run one command headless, read both pipes, and kill it when its limit passes.
/// The first line of a failed command's output is what the record says.
fn run(argv: &[String], extra: &[&str], limit: Duration) -> Result<String, String> {
    let Some(program) = argv.first() else { return Err("the CLI has no program".into()) };
    let mut command = Command::new(program);
    command.args(&argv[1..]).args(extra).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    crate::platform::no_console(&mut command);
    let mut child = command.spawn().map_err(|e| format!("cannot run {program}: {e}"))?;
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let out_thread = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let mut stdout = stdout;
        let _ = stdout.read_to_end(&mut bytes);
        bytes
    });
    let err_thread = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let mut stderr = stderr;
        let _ = stderr.read_to_end(&mut bytes);
        bytes
    });
    let deadline = Instant::now() + limit;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{program} did not end within {} seconds, so it was killed", limit.as_secs()));
            }
        }
    };
    let out = join_reader(out_thread);
    let err = join_reader(err_thread);
    let out = String::from_utf8_lossy(&out).into_owned();
    let err = String::from_utf8_lossy(&err).into_owned();
    if status.success() {
        return Ok(if out.trim().is_empty() { err } else { out });
    }
    Err(first_line(&err)
        .or_else(|| first_line(&out))
        .unwrap_or_else(|| format!("{program} exited with code {}", status.code().unwrap_or(-1))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::clock::{Civil, to_ms};

    const MINUTE: u64 = 60_000;
    const HOUR: u64 = 60 * MINUTE;

    fn entry(tried_ms: u64) -> Entry {
        Entry { tried_ms, ..Default::default() }
    }

    #[test]
    fn an_update_is_due_after_a_day_never_before_the_grace_and_an_hour_after_busy() {
        let started = 1_000;
        assert_eq!(due(None, started, started + 9 * MINUTE, Busy::No), Due::No, "inside the first ten minutes");
        assert_eq!(due(None, started, started + 10 * MINUTE, Busy::No), Due::Now, "never tried");
        let now = started + 26 * HOUR;
        assert_eq!(due(Some(&entry(now - 23 * HOUR)), started, now, Busy::No), Due::No, "23 hours");
        assert_eq!(due(Some(&entry(now - 25 * HOUR)), started, now, Busy::No), Due::Now, "25 hours");
        let busy = Busy::Yes("running now".into());
        assert_eq!(due(Some(&entry(now - 25 * HOUR)), started, now, busy.clone()), Due::Busy);
        assert_eq!(due(None, started, now, busy), Due::Busy, "a never-tried busy CLI waits");
        let mut noted = entry(now - 25 * HOUR);
        noted.busy_ms = now - 30 * MINUTE;
        assert_eq!(due(Some(&noted), started, now, Busy::Yes("running now".into())), Due::No, "not yet an hour");
        noted.busy_ms = now - 61 * MINUTE;
        assert_eq!(due(Some(&noted), started, now, Busy::Yes("running now".into())), Due::Busy);
    }

    #[test]
    fn a_process_name_matches_without_case_or_exe_and_nothing_else_does() {
        assert!(name_matches("OpenCode.EXE", "opencode"));
        assert!(name_matches("GROK.exe", "grok"));
        assert!(name_matches("opencode", "opencode"));
        assert!(!name_matches("opencodex", "opencode"));
        assert!(!name_matches("opencode.cmd", "opencode"));
    }

    fn at(day: u32, hour: u32) -> u64 {
        to_ms(&Civil { year: 2026, month: 11, day, hour, minute: 0, second: 0, weekday: 0 }, 0)
    }

    #[test]
    fn the_line_reads_each_state_and_older_days_as_a_weekday_or_date() {
        let now = at(8, 12);
        let today = at(8, 9);
        assert_eq!(
            line("OpenCode", Some("2.2.6"), Some("2.0.27"), today, today, None, None, now, 0),
            "OpenCode 2.2.6 · updated today (was 2.0.27)"
        );
        assert_eq!(
            line("OpenCode", Some("2.2.6"), Some("2.2.6"), today, 0, None, None, now, 0),
            "OpenCode 2.2.6 · up to date, checked today"
        );
        assert_eq!(
            line("OpenCode", Some("2.0.27"), Some("2.0.27"), today, 0, Some("no network"), None, now, 0),
            "OpenCode 2.0.27 · update failed: no network"
        );
        assert_eq!(
            line("OpenCode", Some("2.0.27"), None, today, 0, None, Some("running now"), now, 0),
            "OpenCode 2.0.27 · waiting: running now"
        );
        assert_eq!(line("OpenCode", None, None, 0, 0, None, None, now, 0), "OpenCode · not checked yet");
        assert_eq!(
            line("OpenCode", Some("2.2.6"), Some("2.0.27"), at(6, 9), at(6, 9), None, None, now, 0),
            "OpenCode 2.2.6 · updated Fri (was 2.0.27)"
        );
        assert_eq!(
            line("OpenCode", Some("2.2.6"), Some("2.0.27"), at(1, 9), at(1, 9), None, None, now, 0),
            "OpenCode 2.2.6 · updated 1 Nov (was 2.0.27)"
        );
        assert_eq!(
            line("Grok Build", Some("1.0.0"), None, at(4, 9), 0, None, None, now, 0),
            "Grok Build 1.0.0 · up to date, checked Wed"
        );
    }

    #[test]
    fn the_record_round_trips_through_updates_json() {
        let dir = std::env::temp_dir().join(format!("kompitum-updates-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut entries = BTreeMap::new();
        entries.insert(
            "opencode".to_string(),
            Entry {
                tried_ms: 10,
                before: Some("2.0.27".into()),
                after: Some("2.2.6".into()),
                updated_ms: 11,
                error: None,
                busy_ms: 0,
            },
        );
        entries.insert("grok".to_string(), Entry { busy_ms: 12, ..Default::default() });
        save(&dir, &entries).unwrap();
        assert_eq!(load(&dir), entries);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
