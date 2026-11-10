//! The vault bell: rings an agent's webhook when a note for it lands in the vault. No model, no quota.
//!
//! Ported from the Python Kompitum's bell (`kompitum-mcp/scripts/kompitum/bell.py`), which is the spec. Each check
//! fetches the vault and, for every target, lists the notes added since its mark under `*/to/<target>/` and
//! `*/to/all/` (its own notes left out). They go out in one POST, and the mark moves only after a 2xx answer.
//! The deciding code (settings, state, which paths a target gets, event lines, the lock decision) stays apart from
//! the IO at the edges: git, curl, the lock file and the clock.

use super::clock::Moment;
use super::git::{self, TIMEOUT};
use crate::platform;
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const DEFAULT_POLL_SECONDS: f64 = 60.0;
const MIN_POLL_SECONDS: f64 = 15.0;
const POST_TIMEOUT_SECONDS: u64 = 30;
const DEFAULT_SOURCE: &str = "vault-doorbell";
const DEFAULT_TARGETS: &str = "manager";

/// What this run was asked to do.
#[derive(Clone, Debug, Default)]
pub struct Options {
    pub once: bool,
    pub dry_run: bool,
    pub status: bool,
    pub env_file: Option<PathBuf>,
}

/// POSTs a body to a URL with a token (empty: no Authorization header). Gives the answer's status, or a short label
/// for a failure that never names the URL or the token.
pub type PostFn = Box<dyn FnMut(&str, &str, &[u8]) -> Result<u16, String>>;

/// One agent the bell rings for, with the settings it was given.
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub name: String,
    pub url: String,
    pub token: String,
    pub source: String,
}

/// The bell's settings, read from the environment: `KOMPITUM_*` first, then the legacy `LIMEN_*`.
#[derive(Clone, Debug)]
pub struct Config {
    pub vault: PathBuf,
    pub targets: Vec<Target>,
    pub poll: f64,
    pub reference: String,
    pub state_file: PathBuf,
}

/// One target's mark: the commit it has seen, when it last rang, and how many notes that ring carried.
#[derive(Clone, Debug, PartialEq)]
pub struct Mark {
    pub commit: String,
    pub rang_at: Option<f64>,
    pub last_count: u64,
}

type State = BTreeMap<String, Mark>;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    Watching,
    Nothing,
    Rang,
    Failed,
    Error,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Http(u16),
    Label(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub target: Option<String>,
    pub action: Action,
    pub count: usize,
    pub status: Option<Status>,
    pub message: String,
}

impl Action {
    fn name(self) -> &'static str {
        match self {
            Action::Watching => "watching",
            Action::Nothing => "nothing",
            Action::Rang => "rang",
            Action::Failed => "failed",
            Action::Error => "error",
        }
    }

    fn is_problem(self) -> bool {
        matches!(self, Action::Failed | Action::Error)
    }
}

fn make_event(target: Option<&str>, action: Action, count: usize, status: Option<Status>, message: &str) -> Event {
    Event { target: target.map(str::to_string), action, count, status, message: message.to_string() }
}

/// The notes a target's check found: what was added, or why that could not be told.
enum Added {
    Notes(Vec<String>),
    /// The mark's commit is not in the history any more, so what was added since is unknown.
    Unknown,
    Failed(String),
}

// ---------------------------------------------------------------------------------------------------------------
// Settings

/// `KOMPITUM_<rest>`, else the legacy `LIMEN_<rest>`; an empty value counts as unset.
fn setting(env: &BTreeMap<String, String>, name: &str) -> Option<String> {
    let rest = name.strip_prefix("KOMPITUM_").unwrap_or(name);
    [format!("KOMPITUM_{rest}"), format!("LIMEN_{rest}")]
        .into_iter()
        .find_map(|key| env.get(&key).filter(|value| !value.is_empty()).cloned())
}

/// A target's name as its variables spell it: `manager-1` becomes `MANAGER_1`.
fn env_key(name: &str) -> String {
    name.to_uppercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect()
}

/// `~` and `~/...` read from the home folder; anything else is as written.
fn expand(raw: &str, home: &Path) -> PathBuf {
    if raw == "~" {
        return home.to_path_buf();
    }
    for prefix in ["~/", "~\\"] {
        if let Some(rest) = raw.strip_prefix(prefix) {
            return home.join(rest);
        }
    }
    PathBuf::from(raw)
}

fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

/// The state file when none is set: `~/.kompitum/<name>`, or the legacy `~/.limen/<name>` while only that exists.
fn default_state_path(home: &Path) -> PathBuf {
    let new = home.join(".kompitum").join("bell-state.json");
    let old = home.join(".limen").join("bell-state.json");
    if !new.exists() && old.exists() { old } else { new }
}

fn parse_seconds(value: &str) -> Result<f64, String> {
    match value.trim().parse::<f64>() {
        Ok(seconds) if seconds.is_finite() && seconds >= MIN_POLL_SECONDS => Ok(seconds),
        _ => Err("KOMPITUM_BELL_POLL_SECONDS is a number of seconds, 15 or more.".into()),
    }
}

/// Reads the bell's settings. The error names the variable that is wrong.
pub fn load_config(env: &BTreeMap<String, String>, home: &Path) -> Result<Config, String> {
    let vault = match setting(env, "KOMPITUM_VAULT") {
        Some(raw) => absolute(&expand(&raw, home)),
        None => home.join("Obsidian").join("vault"),
    };
    if !vault.join(".git").exists() {
        return Err(format!("No Git vault at {}. Set KOMPITUM_VAULT to the vault's folder.", vault.display()));
    }
    let listed = setting(env, "KOMPITUM_BELL_TARGETS").unwrap_or_else(|| DEFAULT_TARGETS.to_string());
    let names: Vec<&str> = listed.split(',').map(str::trim).filter(|name| !name.is_empty()).collect();
    if names.is_empty() {
        return Err("KOMPITUM_BELL_TARGETS is a comma-separated list of agents, and it names none.".into());
    }
    let mut targets: Vec<Target> = Vec::new();
    for name in names {
        if targets.iter().any(|target| target.name == name) {
            continue;
        }
        let key = env_key(name);
        let url = setting(env, &format!("KOMPITUM_BELL_{key}_URL")).unwrap_or_default().trim().to_string();
        if url.is_empty() {
            return Err(format!("KOMPITUM_BELL_{key}_URL is missing. Set it to {name}'s webhook URL."));
        }
        let token = setting(env, &format!("KOMPITUM_BELL_{key}_TOKEN")).unwrap_or_default().trim().to_string();
        let source = setting(env, &format!("KOMPITUM_BELL_{key}_SOURCE")).unwrap_or_default().trim().to_string();
        let source = if source.is_empty() { DEFAULT_SOURCE.to_string() } else { source };
        targets.push(Target { name: name.to_string(), url, token, source });
    }
    let state_file = match setting(env, "KOMPITUM_BELL_STATE") {
        Some(raw) => absolute(&expand(&raw, home)),
        None => default_state_path(home),
    };
    let poll =
        parse_seconds(&setting(env, "KOMPITUM_BELL_POLL_SECONDS").unwrap_or_else(|| DEFAULT_POLL_SECONDS.to_string()))?;
    let reference = setting(env, "KOMPITUM_BELL_REF").unwrap_or_default().trim().to_string();
    Ok(Config { vault, targets, poll, reference, state_file })
}

/// `KEY=VALUE` lines: blank lines and `#` comments skipped, one pair of matching quotes around a value removed.
fn parse_env_file(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let (key, value) = line.split_once('=')?;
            let key = key.trim();
            let mut value = value.trim();
            if key.is_empty() {
                return None;
            }
            let quoted = value.len() >= 2
                && ((value.starts_with('"') && value.ends_with('"'))
                    || (value.starts_with('\'') && value.ends_with('\'')));
            if quoted {
                value = &value[1..value.len() - 1];
            }
            Some((key.to_string(), value.to_string()))
        })
        .collect()
}

/// Adds each pair the environment does not already set: a variable already set keeps its value.
fn apply_env_file(env: &mut BTreeMap<String, String>, pairs: Vec<(String, String)>) {
    for (key, value) in pairs {
        env.entry(key).or_insert(value);
    }
}

fn read_env_file(path: &Path) -> Result<Vec<(String, String)>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("--env-file {}: {:?}", path.display(), e.kind()))?;
    Ok(parse_env_file(&String::from_utf8_lossy(&bytes)))
}

// ---------------------------------------------------------------------------------------------------------------
// State

/// The marks in the state file, or None when the file is not a valid set of marks.
fn parse_state(text: &str) -> Option<State> {
    let parsed: Value = serde_json::from_str(text).ok()?;
    let mut state = State::new();
    for (name, mark) in parsed.as_object()? {
        let commit = mark.get("commit")?.as_str().filter(|commit| !commit.is_empty())?;
        state.insert(
            name.clone(),
            Mark {
                commit: commit.to_string(),
                rang_at: mark.get("rang_at").and_then(Value::as_f64),
                last_count: mark.get("last_count").and_then(Value::as_u64).unwrap_or(0),
            },
        );
    }
    Some(state)
}

fn state_text(state: &State) -> String {
    let object: Map<String, Value> = state
        .iter()
        .map(|(name, mark)| {
            (name.clone(), json!({"commit": mark.commit, "rang_at": mark.rang_at, "last_count": mark.last_count}))
        })
        .collect();
    let mut text = serde_json::to_string_pretty(&Value::Object(object)).unwrap_or_default();
    text.push('\n');
    text
}

/// The marks, and a problem worth one error line when the file is unreadable or corrupt.
fn read_state(file: &Path) -> (State, Option<String>) {
    match std::fs::read(file) {
        Ok(bytes) => match parse_state(&String::from_utf8_lossy(&bytes)) {
            Some(state) => (state, None),
            None => {
                (State::new(), Some(format!("the state file {} is corrupt, so the bell starts empty", file.display())))
            }
        },
        Err(e) if e.kind() == ErrorKind::NotFound => (State::new(), None),
        Err(e) => (
            State::new(),
            Some(format!(
                "the state file {} could not be read ({:?}), so the bell starts empty",
                file.display(),
                e.kind()
            )),
        ),
    }
}

/// Written whole and private to its user, as the Python bell's was: a private temp file renamed over it.
fn write_state(file: &Path, state: &State) -> std::io::Result<()> {
    if let Some(folder) = file.parent() {
        std::fs::create_dir_all(folder)?;
    }
    let name = file.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let temp = file.with_file_name(format!("{name}.{}.tmp", std::process::id()));
    let _ = std::fs::remove_file(&temp);
    let written = (|| {
        let mut out = platform::create_private(&temp)?;
        out.write_all(state_text(state).as_bytes())?;
        out.sync_all()?;
        drop(out);
        std::fs::rename(&temp, file)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    written
}

// ---------------------------------------------------------------------------------------------------------------
// Which notes a target gets, and the POST body

/// The paths a target is rung for: `*/to/<target>/...` and `*/to/all/...`, its own notes left out, sorted.
fn kept(target: &str, listing: &str) -> Vec<String> {
    let paths: BTreeSet<&str> = listing
        .split('\0')
        .filter(|path| {
            let parts: Vec<&str> = path.split('/').collect();
            parts.len() >= 4 && parts[1] == "to" && (parts[2] == target || parts[2] == "all") && parts[0] != target
        })
        .collect();
    paths.into_iter().map(str::to_string).collect()
}

/// The POST body: `{"source": ..., "new_notes": [...]}`, plus `"mark_reset": true` after an unknown mark.
fn payload(source: &str, notes: &[String], reset: bool) -> String {
    let mut body = format!("{{\"source\":{},\"new_notes\":{}", json!(source), json!(notes));
    if reset {
        body.push_str(",\"mark_reset\":true");
    }
    body.push('}');
    body
}

fn short(commit: &str) -> &str {
    commit.get(..7).unwrap_or(commit)
}

fn plural(count: usize) -> &'static str {
    if count == 1 { "note" } else { "notes" }
}

/// One event as the log shows it. Rings and failures carry their count and status; the rest carry their message.
pub fn line(event: &Event, stamp: Option<&str>) -> String {
    let prefix = stamp.map(|stamp| format!("{stamp} ")).unwrap_or_default();
    let who = match &event.target {
        Some(name) => format!("bell {name}"),
        None => "bell".to_string(),
    };
    match event.action {
        Action::Rang | Action::Failed => {
            let mut text = format!("{prefix}{who}: {} {} {}", event.action.name(), event.count, plural(event.count));
            match &event.status {
                Some(Status::Http(code)) => text.push_str(&format!(" (HTTP {code})")),
                Some(Status::Label(label)) if !label.is_empty() => text.push_str(&format!(" ({label})")),
                _ => {}
            }
            text
        }
        action => {
            let mut text = format!("{prefix}{who}: {}", action.name());
            if !event.message.is_empty() {
                text.push(' ');
                text.push_str(&event.message);
            }
            text
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The check

/// The bell's state for one run: its settings, a POST function and the clock.
pub struct Bell {
    config: Config,
    clock: fn() -> f64,
    post: PostFn,
    state_reported: bool,
}

impl Bell {
    pub fn new(config: Config, clock: fn() -> f64, post: PostFn) -> Bell {
        Bell { config, clock, post, state_reported: false }
    }

    /// One check: fetch, resolve the ref, then ring each target on its own.
    pub fn tick(&mut self) -> Result<Vec<Event>, String> {
        let (mut state, mut events) = self.read_marks();
        let new = match self.locate() {
            Ok(commit) => commit,
            Err(failure) => {
                events.push(failure);
                return Ok(events);
            }
        };
        for target in self.config.targets.clone() {
            events.extend(self.check(&target, &new, &mut state)?);
        }
        Ok(events)
    }

    /// What a check would ring, without posting or saving: the notes per target, and any errors.
    pub fn plan(&mut self) -> (Vec<(String, Vec<String>)>, Vec<Event>) {
        let (state, mut events) = self.read_marks();
        let new = match self.locate() {
            Ok(commit) => commit,
            Err(failure) => {
                events.push(failure);
                return (Vec::new(), events);
            }
        };
        let mut planned = Vec::new();
        for target in self.config.targets.clone() {
            let Some(mark) = state.get(&target.name) else { continue };
            if mark.commit == new {
                continue;
            }
            match self.added(&target.name, &mark.commit, &new) {
                Added::Unknown => events.push(make_event(
                    Some(&target.name),
                    Action::Error,
                    0,
                    None,
                    &format!(
                        "the mark commit is unknown; a tick would ring the whole inbox and reset it to {}",
                        short(&new)
                    ),
                )),
                Added::Failed(problem) => events.push(make_event(
                    Some(&target.name),
                    Action::Error,
                    0,
                    None,
                    &format!("the diff failed: {problem}"),
                )),
                Added::Notes(notes) if !notes.is_empty() => planned.push((target.name.clone(), notes)),
                Added::Notes(_) => {}
            }
        }
        (planned, events)
    }

    fn git(&self, args: &[&str]) -> git::GitResult {
        git::run(&self.config.vault, args, TIMEOUT)
    }

    fn read_marks(&mut self) -> (State, Vec<Event>) {
        let (state, problem) = read_state(&self.config.state_file);
        let mut events = Vec::new();
        if let Some(problem) = problem
            && !self.state_reported
        {
            self.state_reported = true;
            events.push(make_event(None, Action::Error, 0, None, &problem));
        }
        (state, events)
    }

    /// Fetches, then resolves the ref to the commit it names.
    fn locate(&self) -> Result<String, Event> {
        let fetched = self.git(&["fetch", "--quiet"]);
        if !fetched.ok {
            return Err(make_event(None, Action::Error, 0, None, &format!("the fetch failed: {}", fetched.error)));
        }
        let reference: &str = if self.config.reference.is_empty() { "@{upstream}" } else { &self.config.reference };
        let spec = format!("{reference}^{{commit}}");
        let resolved = self.git(&["rev-parse", "--verify", spec.as_str()]);
        if !resolved.ok {
            return Err(make_event(
                None,
                Action::Error,
                0,
                None,
                &format!("the ref {reference} does not resolve to a commit; set KOMPITUM_BELL_REF"),
            ));
        }
        Ok(resolved.output.trim().to_string())
    }

    fn check(&mut self, target: &Target, new: &str, state: &mut State) -> Result<Vec<Event>, String> {
        let name = target.name.as_str();
        let Some(mark) = state.get(name).cloned() else {
            // The first look only watches: a backlog that was already there never rings.
            state.insert(name.to_string(), Mark { commit: new.to_string(), rang_at: None, last_count: 0 });
            write_state(&self.config.state_file, state).map_err(save_failure)?;
            return Ok(vec![make_event(Some(name), Action::Watching, 0, None, &format!("from {}", short(new)))]);
        };
        if mark.commit == new {
            return Ok(vec![make_event(Some(name), Action::Nothing, 0, None, "")]);
        }
        let (notes, reset) = match self.added(name, &mark.commit, new) {
            Added::Notes(notes) => (notes, false),
            // The history moved under the mark: ring the whole inbox at the tip, flagged, and move the mark only
            // after a 2xx (the Python's AU NB-2).
            Added::Unknown => match self.inbox(name, new) {
                Ok(notes) => (notes, true),
                Err(problem) => {
                    return Ok(vec![make_event(
                        Some(name),
                        Action::Error,
                        0,
                        None,
                        &format!("the diff failed: {problem}"),
                    )]);
                }
            },
            Added::Failed(problem) => {
                return Ok(vec![make_event(
                    Some(name),
                    Action::Error,
                    0,
                    None,
                    &format!("the diff failed: {problem}"),
                )]);
            }
        };
        if notes.is_empty() {
            state.insert(name.to_string(), Mark { commit: new.to_string(), ..mark });
            write_state(&self.config.state_file, state).map_err(save_failure)?;
            return Ok(vec![make_event(Some(name), Action::Nothing, 0, None, "")]);
        }
        let body = payload(&target.source, &notes, reset);
        let count = notes.len();
        match (self.post)(&target.url, &target.token, body.as_bytes()) {
            Err(label) => Ok(vec![make_event(
                Some(name),
                Action::Failed,
                count,
                Some(Status::Label(label)),
                "the POST failed, so the mark stays",
            )]),
            Ok(code) if (200..300).contains(&code) => {
                let message = if reset {
                    format!(
                        "the mark commit was unknown, so the whole inbox rang and the mark was reset to {}",
                        short(new)
                    )
                } else {
                    String::new()
                };
                let rang_at = Some((self.clock)());
                state.insert(name.to_string(), Mark { commit: new.to_string(), rang_at, last_count: count as u64 });
                write_state(&self.config.state_file, state).map_err(save_failure)?;
                Ok(vec![make_event(Some(name), Action::Rang, count, Some(Status::Http(code)), &message)])
            }
            Ok(code) => Ok(vec![make_event(
                Some(name),
                Action::Failed,
                count,
                Some(Status::Http(code)),
                "the webhook did not answer 2xx, so the mark stays",
            )]),
        }
    }

    /// The commits added since the mark, through git. `--no-renames`: a note moved into an inbox counts as added.
    fn added(&self, target: &str, mark: &str, new: &str) -> Added {
        let diff = self.git(&["diff", "--name-only", "--no-renames", "--diff-filter=A", "-z", mark, new]);
        if !diff.ok {
            let known = self.git(&["cat-file", "-e", &format!("{mark}^{{commit}}")]);
            return if known.ok { Added::Failed(diff.error) } else { Added::Unknown };
        }
        Added::Notes(kept(target, &diff.output))
    }

    fn inbox(&self, target: &str, new: &str) -> Result<Vec<String>, String> {
        let listed = self.git(&["ls-tree", "-r", "--name-only", "-z", new]);
        if !listed.ok {
            return Err(listed.error);
        }
        Ok(kept(target, &listed.output))
    }
}

fn save_failure(error: std::io::Error) -> String {
    format!("the state file could not be saved ({:?})", error.kind())
}

// ---------------------------------------------------------------------------------------------------------------
// The POST, through curl. The URL and the token go in a config file, never in curl's argument list.

/// curl's arguments: its config file is read, the body comes on stdin, and nothing is followed (no `-L`).
fn curl_args(config: &Path) -> Vec<String> {
    let timeout = POST_TIMEOUT_SECONDS.to_string();
    let mut args: Vec<String> = [
        "--disable",
        "--silent",
        "--show-error",
        "--max-time",
        timeout.as_str(),
        "--output",
        platform::null_device(),
        "--write-out",
        "%{http_code}",
        "--config",
    ]
    .iter()
    .map(|arg| arg.to_string())
    .collect();
    args.push(config.to_string_lossy().into_owned());
    args.extend(["--data-binary", "@-"].map(String::from));
    args
}

/// The curl config: the URL, the JSON content type and, when there is one, the bearer token.
fn curl_config(url: &str, token: &str) -> String {
    let mut text = format!("url = {}\nheader = \"Content-Type: application/json\"\n", quoted(url));
    if !token.is_empty() {
        text.push_str(&format!("header = {}\n", quoted(&format!("Authorization: Bearer {token}"))));
    }
    text
}

fn quoted(text: &str) -> String {
    let escaped = text.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n").replace('\r', "\\r");
    format!("\"{escaped}\"")
}

/// The real POST: a private config file for curl, removed whatever happened.
fn curl_post(url: &str, token: &str, body: &[u8]) -> Result<u16, String> {
    let file = std::env::temp_dir().join(format!("kom-bell-{}-{}.curl", std::process::id(), platform::random_hex()));
    let written = (|| -> std::io::Result<()> {
        let mut out = platform::create_private(&file)?;
        out.write_all(curl_config(url, token).as_bytes())
    })();
    let result = match written {
        Ok(()) => run_curl(&file, body),
        Err(e) => Err(format!("the curl config could not be written ({:?})", e.kind())),
    };
    let _ = std::fs::remove_file(&file);
    result
}

fn run_curl(config: &Path, body: &[u8]) -> Result<u16, String> {
    let mut cmd = Command::new("curl");
    cmd.args(curl_args(config)).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    platform::no_console(&mut cmd);
    let mut child = cmd.spawn().map_err(|e| format!("curl did not start ({:?})", e.kind()))?;
    let mut stdin = child.stdin.take().expect("stdin is piped");
    let body = body.to_vec();
    let feeder = std::thread::spawn(move || {
        let _ = stdin.write_all(&body);
    });
    let output = child.wait_with_output();
    let _ = feeder.join();
    let output = output.map_err(|e| format!("curl did not finish ({:?})", e.kind()))?;
    if !output.status.success() {
        return Err(format!("curl exit {}", output.status.code().unwrap_or(-1)));
    }
    String::from_utf8_lossy(&output.stdout).trim().parse::<u16>().map_err(|_| "curl gave no status code".to_string())
}

// ---------------------------------------------------------------------------------------------------------------
// The single-instance lock: a file beside the state file naming its owner's pid and the boot it ran in.

pub struct Lock {
    path: PathBuf,
    token: String,
}

fn lock_path(state_file: &Path) -> PathBuf {
    let mut name = state_file.as_os_str().to_owned();
    name.push(".lock");
    PathBuf::from(name)
}

/// The lock's owner: its pid (when it is a usable one) and its boot id (when it names one).
fn parse_owner(text: &str) -> (Option<u32>, Option<String>) {
    let Ok(held) = serde_json::from_str::<Value>(text) else { return (None, None) };
    let pid = held
        .get("pid")
        .and_then(Value::as_u64)
        .and_then(|pid| u32::try_from(pid).ok())
        .filter(|pid| *pid > 0 && i32::try_from(*pid).is_ok());
    let boot = held.get("boot_id").and_then(Value::as_str).filter(|boot| !boot.is_empty()).map(str::to_string);
    (pid, boot)
}

/// Whether a bell command line is one of ours: it names `bell` and a kom or the legacy limen binary.
fn is_bell_argv(argv: &[String]) -> bool {
    argv.iter().any(|arg| arg == "bell")
        && argv.iter().any(|arg| {
            let name = Path::new(arg).file_name().map(|name| name.to_string_lossy().to_lowercase()).unwrap_or_default();
            name.contains("kom") || name.contains("limen")
        })
}

/// Whether the pid runs a bell now. Where the OS shows no argument list, a live pid counts.
fn pid_runs_bell(pid: u32) -> bool {
    platform::process_alive(pid) && platform::process_argv(pid).is_none_or(|argv| is_bell_argv(&argv))
}

/// Whether a held lock still blocks. A lock from another boot is stale even when its pid lives on.
fn lock_is_live(lock_boot: Option<&str>, boot: Option<&str>, runs_bell: bool) -> bool {
    match (lock_boot, boot) {
        (Some(held), Some(now)) if held != now => false,
        _ => runs_bell,
    }
}

/// The lock, or None while a live bell in this boot holds it. A lock that cannot be read, that names a dead
/// process or another boot, or that names a process which is no longer a bell is taken over.
pub fn acquire_lock(state_file: &Path) -> std::io::Result<Option<Lock>> {
    let path = lock_path(state_file);
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder)?;
    }
    for _ in 0..3 {
        match platform::create_private(&path) {
            Ok(mut file) => {
                let token = format!("{}-{}", std::process::id(), platform::random_hex());
                let held = json!({"pid": std::process::id(), "boot_id": platform::boot_id(), "token": token});
                if let Err(e) = file.write_all(held.to_string().as_bytes()) {
                    let _ = std::fs::remove_file(&path);
                    return Err(e);
                }
                return Ok(Some(Lock { path, token }));
            }
            Err(e) if e.kind() == ErrorKind::AlreadyExists => {
                let (owner, owner_boot) = parse_owner(&std::fs::read_to_string(&path).unwrap_or_default());
                if owner == Some(std::process::id()) {
                    return Ok(None);
                }
                if let Some(pid) = owner
                    && lock_is_live(owner_boot.as_deref(), platform::boot_id().as_deref(), pid_runs_bell(pid))
                {
                    return Ok(None);
                }
                let _ = std::fs::remove_file(&path);
            }
            Err(e) => return Err(e),
        }
    }
    Ok(None)
}

pub fn release_lock(lock: Lock) {
    let held = std::fs::read_to_string(&lock.path).ok().and_then(|text| serde_json::from_str::<Value>(&text).ok());
    if held.as_ref().and_then(|held| held.get("token")).and_then(Value::as_str) == Some(lock.token.as_str()) {
        let _ = std::fs::remove_file(&lock.path);
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Running

fn wall_clock() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |delta| delta.as_secs_f64())
}

/// The local time to the second, as the log writes it: `2026-11-01 04:00:00`.
fn now_stamp() -> String {
    let stamp = Moment::now().mark_stamp();
    stamp.get(..19).unwrap_or(&stamp).replace('T', " ")
}

/// Keeps the URL and the token out of anything printed, whatever the failure said.
fn safe(text: String, config: &Config) -> String {
    config.targets.iter().fold(text, |text, target| {
        let text = if target.url.is_empty() { text } else { text.replace(&target.url, "<url>") };
        if target.token.is_empty() { text } else { text.replace(&target.token, "<token>") }
    })
}

fn upstream(vault: &Path) -> Option<String> {
    let found = git::run(vault, &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{upstream}"], TIMEOUT);
    found.ok.then(|| found.output.trim().to_string())
}

fn status_line(config: &Config) -> String {
    let (state, _) = read_state(&config.state_file);
    let targets: Map<String, Value> = config
        .targets
        .iter()
        .map(|target| {
            let mark = state.get(&target.name);
            (
                target.name.clone(),
                json!({
                    "url": if target.url.is_empty() { "missing" } else { "set" },
                    "token": if target.token.is_empty() { "unset" } else { "set" },
                    "commit": mark.map(|mark| mark.commit.clone()),
                    "rang_at": mark.and_then(|mark| mark.rang_at),
                }),
            )
        })
        .collect();
    let reference = if config.reference.is_empty() { upstream(&config.vault) } else { Some(config.reference.clone()) };
    let poll = if config.poll.fract() == 0.0 { json!(config.poll as i64) } else { json!(config.poll) };
    json!({
        "vault": config.vault.display().to_string(),
        "ref": reference,
        "poll_seconds": poll,
        "targets": targets,
    })
    .to_string()
}

/// The bell's entry point: prints its lines through `out`, its errors through `err`, and returns the exit code.
/// Codes: 0 fine, 1 a ring failed or the check failed, 2 the settings are wrong.
pub fn run(options: &Options) -> i32 {
    let mut env: BTreeMap<String, String> = std::env::vars().collect();
    if let Some(path) = &options.env_file {
        match read_env_file(path) {
            Ok(pairs) => apply_env_file(&mut env, pairs),
            Err(problem) => {
                eprintln!("error: {problem}");
                return 2;
            }
        }
    }
    let home = platform::home_dir();
    drive(&env, &home, options, Box::new(curl_post), &mut |line| println!("{line}"), &mut |line| {
        eprintln!("error: {line}")
    })
}

fn drive(
    env: &BTreeMap<String, String>,
    home: &Path,
    options: &Options,
    post: PostFn,
    out: &mut dyn FnMut(&str),
    err: &mut dyn FnMut(&str),
) -> i32 {
    let config = match load_config(env, home) {
        Ok(config) => config,
        Err(problem) => {
            err(&problem);
            return 2;
        }
    };
    if options.status {
        out(&status_line(&config));
        return 0;
    }
    let mut bell = Bell::new(config, wall_clock, post);
    if options.dry_run {
        return run_dry(&mut bell, out);
    }
    let lock = match acquire_lock(&bell.config.state_file) {
        Ok(Some(lock)) => lock,
        Ok(None) => {
            out(&format!(
                "bell: another bell is already running on {}; this one exits",
                bell.config.state_file.display()
            ));
            return 0;
        }
        Err(e) => {
            err(&format!("the bell's lock could not be taken ({:?})", e.kind()));
            return 1;
        }
    };
    let code = if options.once { run_once(&mut bell, out, err) } else { run_loop(&mut bell, out) };
    release_lock(lock);
    code
}

fn run_once(bell: &mut Bell, out: &mut dyn FnMut(&str), err: &mut dyn FnMut(&str)) -> i32 {
    match bell.tick() {
        Err(problem) => {
            err(&safe(problem, &bell.config));
            1
        }
        Ok(events) => {
            let stamp = now_stamp();
            for event in &events {
                out(&safe(line(event, Some(&stamp)), &bell.config));
            }
            i32::from(events.iter().any(|event| event.action.is_problem()))
        }
    }
}

fn run_dry(bell: &mut Bell, out: &mut dyn FnMut(&str)) -> i32 {
    let (planned, events) = bell.plan();
    for (target, paths) in &planned {
        out(&format!("would ring {target}: {} {}", paths.len(), plural(paths.len())));
        for path in paths {
            out(path);
        }
    }
    for event in &events {
        out(&safe(line(event, None), &bell.config));
    }
    i32::from(events.iter().any(|event| event.action.is_problem()))
}

/// Checks, prints what happened (but not the quiet "nothing"), waits the poll interval, and goes again. Until killed.
fn run_loop(bell: &mut Bell, out: &mut dyn FnMut(&str)) -> i32 {
    loop {
        match bell.tick() {
            Ok(events) => {
                let stamp = now_stamp();
                for event in events.iter().filter(|event| event.action != Action::Nothing) {
                    out(&safe(line(event, Some(&stamp)), &bell.config));
                }
            }
            Err(problem) => out(&safe(format!("{} bell: error {problem}", now_stamp()), &bell.config)),
        }
        std::thread::sleep(Duration::from_secs_f64(bell.config.poll));
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Tests live beside this file, in bell/tests.rs.

#[cfg(test)]
mod tests;
