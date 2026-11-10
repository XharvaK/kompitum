//! The only place that knows a worker CLI by name: how to find it, how to ask
//! its version, how to tell it to run a turn, and where its reader lives.
//! The streams themselves are in `opencode.rs`, `commandcode.rs` and `codex.rs`.

use crate::config::Config;
use crate::rpc::RpcError;
use crate::task::types::Usage;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;
use std::time::Duration;

/// A worker CLI the loop can run headless and read. Its launch, stream and quota words live in code.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Cli {
    Codex,
    CommandCode,
    Opencode,
    Claude,
}

impl Cli {
    pub fn key(self) -> &'static str {
        match self {
            Cli::Codex => "codex",
            Cli::CommandCode => "command_code",
            Cli::Opencode => "opencode",
            Cli::Claude => "claude",
        }
    }

    pub fn from_key(key: &str) -> Option<Cli> {
        match key {
            "codex" => Some(Cli::Codex),
            "command_code" => Some(Cli::CommandCode),
            "opencode" => Some(Cli::Opencode),
            "claude" => Some(Cli::Claude),
            _ => None,
        }
    }
}

/// One worker: a CLI and the model route it runs.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
pub struct Worker {
    pub key: String,
    pub label: String,
    pub cli: Cli,
    pub route: String,
}

/// The workers every core has, unchanged.
pub fn builtin() -> Vec<Worker> {
    vec![
        Worker {
            key: "cc".into(),
            label: "Command Code".into(),
            cli: Cli::CommandCode,
            route: "commandcode/deepseek/deepseek-v4.1-flash#max".into(),
        },
        Worker {
            key: "go".into(),
            label: "opencode Go".into(),
            cli: Cli::Opencode,
            route: "opencode-go/deepseek-v4.1-flash#max".into(),
        },
        Worker {
            key: "free".into(),
            label: "opencode free".into(),
            cli: Cli::Opencode,
            route: "opencode/deepseek/deepseek-v4.1-flash-free#max".into(),
        },
        Worker { key: "codex".into(), label: "Codex".into(), cli: Cli::Codex, route: "gpt-6-luna#high".into() },
        Worker {
            key: "haiku".into(),
            label: "Claude Haiku".into(),
            cli: Cli::Claude,
            route: "claude-haiku-5-5#high".into(),
        },
    ]
}

/// The label for a stored worker key; an unknown key stays as it is.
pub fn label_of(workers: &[Worker], key: &str) -> String {
    workers
        .iter()
        .chain(builtin().iter())
        .find(|worker| worker.key == key)
        .map_or_else(|| key.to_string(), |worker| worker.label.clone())
}

impl Worker {
    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn route(&self) -> &str {
        &self.route
    }
}

fn worker_missing(cli: &str, key: &str, message: impl Into<String>) -> RpcError {
    RpcError::new(
        "worker_missing",
        message,
        format!("install {cli}, or set [workers] {key} in ~/.config/kompitum-core/config.toml"),
    )
}

/// A shell shim can't be spawned without a shell, and the real executable's
/// output is what the loop reads: refuse it by name, as the Python does.
fn check_shim(cli: &str, key: &str, exe: &str) -> Result<(), RpcError> {
    let lower = exe.to_ascii_lowercase();
    if [".cmd", ".bat", ".ps1"].iter().any(|ext| lower.ends_with(ext)) {
        return Err(worker_missing(cli, key, format!("{exe:?} is a shell shim: give the {cli} executable itself")));
    }
    Ok(())
}

/// A program's file name on this system. On Windows only the `.exe` starts
/// without a shell: npm leaves a shell script called `opencode` beside it.
pub(crate) fn program(name: &str) -> String {
    format!("{name}{}", std::env::consts::EXE_SUFFIX)
}

pub(crate) fn on_path(names: &[String]) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
        .find(|candidate| candidate.is_file())
}

fn python() -> Option<Vec<String>> {
    // The Windows Store's alias is a file on PATH that only prints an advert.
    let names = [program("python3"), program("python")];
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .filter(|dir| !dir.to_string_lossy().trim_end_matches('\\').to_ascii_lowercase().ends_with("\\windowsapps"))
        .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
        .find(|candidate| candidate.is_file())
        .map(|path| vec![path.display().to_string()])
}

fn node() -> Option<PathBuf> {
    on_path(&[program("node")])
}

/// An explicit command is installed when its program is a file or on PATH.
fn require_program(cli: &str, key: &str, exe: &str) -> Result<(), RpcError> {
    if Path::new(exe).is_file() || on_path(&[program(exe), exe.to_string()]).is_some() {
        return Ok(());
    }
    Err(worker_missing(cli, key, format!("{cli} was not found: {exe:?} is not a file or on PATH")))
}

/// An explicit argv, run through python or node when it is a script, never
/// through a shell shim.
fn command_program(cli: &str, key: &str, mut argv: Vec<String>) -> Result<Vec<String>, RpcError> {
    let Some(exe) = argv.first().cloned() else {
        return Err(worker_missing(cli, key, format!("[workers] {key} is an empty command")));
    };
    check_shim(cli, key, &exe)?;
    require_program(cli, key, &exe)?;
    let lower = exe.to_ascii_lowercase();
    if lower.ends_with(".py") {
        let mut python =
            python().ok_or_else(|| worker_missing(cli, key, format!("no python to run {cli}'s script")))?;
        python.append(&mut argv);
        return Ok(python);
    }
    if lower.ends_with(".mjs") || lower.ends_with(".js") {
        let node = node().ok_or_else(|| worker_missing(cli, key, format!("no node to run {cli}'s script")))?;
        let mut out = vec![node.display().to_string()];
        out.append(&mut argv);
        return Ok(out);
    }
    Ok(argv)
}

/// `opencode`'s argv prefix: `[workers] opencode` when the config gives one,
/// else the executable on PATH (or the npm fallback on Windows), as
/// `command_prefix` does in the Python.
pub(crate) fn opencode_prefix(config: &Config) -> Result<Vec<String>, RpcError> {
    if let Some(argv) = config.opencode() {
        return command_program("OpenCode v2", "opencode", argv);
    }
    let found = on_path(&[program("opencode")]).or_else(|| {
        let appdata = std::env::var_os("APPDATA")?;
        let fallback = PathBuf::from(appdata)
            .join("npm")
            .join("node_modules")
            .join("@opencode")
            .join("cli")
            .join("bin")
            .join("opencode.exe");
        fallback.is_file().then_some(fallback)
    });
    match found {
        Some(found) => Ok(vec![found.display().to_string()]),
        None => Err(worker_missing("OpenCode v2", "opencode", "OpenCode was not found on PATH")),
    }
}

/// `commandcode_prefix`: an explicit argv, else the npm install under node,
/// else `cmdc`, then `commandcode`, on PATH.
fn commandcode_prefix(config: &Config) -> Result<Vec<String>, RpcError> {
    if let Some(argv) = config.commandcode() {
        return command_program("Command Code", "commandcode", argv);
    }
    if let Some(appdata) = std::env::var_os("APPDATA") {
        let candidate =
            PathBuf::from(appdata).join("npm").join("node_modules").join("command-code").join("dist").join("index.mjs");
        if candidate.is_file()
            && let Some(node) = node()
        {
            return Ok(vec![node.display().to_string(), candidate.display().to_string()]);
        }
    }
    for name in ["cmdc", "commandcode"] {
        if let Some(found) = on_path(&[program(name)]) {
            return Ok(vec![found.display().to_string()]);
        }
    }
    Err(worker_missing("Command Code", "commandcode", "Command Code was not found on PATH"))
}

/// `codex_prefix`: an explicit argv, else the executable on PATH.
pub(crate) fn codex_prefix(config: &Config) -> Result<Vec<String>, RpcError> {
    if let Some(argv) = config.codex() {
        return command_program("Codex", "codex", argv);
    }
    match on_path(&[program("codex")]) {
        Some(found) => Ok(vec![found.display().to_string()]),
        None => Err(worker_missing("Codex", "codex", "Codex was not found on PATH")),
    }
}

/// `claude_prefix`: an explicit argv, else the executable on PATH.
pub(crate) fn claude_prefix(config: &Config) -> Result<Vec<String>, RpcError> {
    if let Some(argv) = config.claude() {
        return command_program("Claude Code", "claude", argv);
    }
    match on_path(&[program("claude")]) {
        Some(found) => Ok(vec![found.display().to_string()]),
        None => Err(worker_missing("Claude Code", "claude", "Claude Code was not found on PATH")),
    }
}

/// The argv prefix that starts a worker CLI.
pub fn prefix(worker: &Worker, config: &Config) -> Result<Vec<String>, RpcError> {
    match worker.cli {
        Cli::Codex => codex_prefix(config),
        Cli::CommandCode => commandcode_prefix(config),
        Cli::Opencode => opencode_prefix(config),
        Cli::Claude => claude_prefix(config),
    }
}

/// Prefixes whose version this core already accepted: asking once is enough.
static CHECKED: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

pub(crate) fn first_version(text: &str) -> Option<(u32, u32, u32)> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            let mut parts = ["", "", ""];
            let mut count = 0;
            loop {
                let from = i;
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                parts[count] = &text[from..i];
                count += 1;
                if count == 3 || i >= bytes.len() || bytes[i] != b'.' {
                    break;
                }
                i += 1;
                if i >= bytes.len() || !bytes[i].is_ascii_digit() {
                    break;
                }
            }
            if count == 3
                && let (Ok(a), Ok(b), Ok(c)) = (parts[0].parse(), parts[1].parse(), parts[2].parse())
            {
                return Some((a, b, c));
            }
            i = start + 1;
        } else {
            i += 1;
        }
    }
    None
}

/// `check_opencode_v2`: one `--version` per core per prefix, 30 s at most.
pub async fn check_version(prefix: &[String]) -> Result<(), RpcError> {
    let key = prefix.join("\u{0}");
    if CHECKED.lock().unwrap_or_else(|p| p.into_inner()).contains(&key) {
        return Ok(());
    }
    let mut command = std::process::Command::new(&prefix[0]);
    command.args(&prefix[1..]).arg("--version").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    crate::platform::no_console(&mut command);
    let mut cmd = tokio::process::Command::from(command);
    cmd.kill_on_drop(true);
    let out = match tokio::time::timeout(Duration::from_secs(30), cmd.output()).await {
        Ok(Ok(out)) => out,
        Ok(Err(e)) => {
            return Err(worker_missing("OpenCode v2", "opencode", format!("cannot run {} --version: {e}", prefix[0])));
        }
        Err(_) => {
            return Err(worker_missing(
                "OpenCode v2",
                "opencode",
                format!("{} --version did not answer within 30 s", prefix[0]),
            ));
        }
    };
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    match first_version(&text) {
        Some((2, _, _)) => {
            CHECKED.lock().unwrap_or_else(|p| p.into_inner()).insert(key);
            Ok(())
        }
        _ => {
            let found = crate::task::types::cut(text.trim(), 120);
            Err(worker_missing(
                "OpenCode v2",
                "opencode",
                format!(
                    "kom requires OpenCode v2.x; found: {}",
                    if found.is_empty() { "nothing".into() } else { found }
                ),
            ))
        }
    }
}

/// The whole command for one OpenCode turn. `session` means continue that conversation.
pub fn opencode_command(
    prefix: &[String],
    worker: &Worker,
    task: &str,
    prompt_path: &str,
    session: Option<&str>,
) -> Vec<String> {
    let mut argv = prefix.to_vec();
    // `--standalone` keeps the loop out of any server the person's TUI runs;
    // `--format json` is what the runner reads; `--auto` never asks a question.
    argv.extend(["run", "--standalone", "--auto", "--format", "json", "--model", worker.route()].map(String::from));
    if let Some(session) = session {
        // A continuation keeps its name: a manual rename in the TUI is not clobbered.
        argv.extend(["--session".to_string(), session.to_string()]);
    } else {
        argv.extend(["--title".to_string(), format!("kompitum-{task}")]);
    }
    // OpenCode v2 hangs on an open stdin, and a long brief hits argv limits on
    // Windows: the brief travels as a file, and the runner closes stdin.
    argv.extend([
        "--file".to_string(),
        prompt_path.to_string(),
        "--".to_string(),
        "The attached file is the full task brief. Read it and carry it out exactly.".to_string(),
    ]);
    argv
}

/// Everything the runner needs to start one turn.
pub struct TurnCommand {
    pub argv: Vec<String>,
    /// The file the program reads as stdin. None: stdin is closed.
    pub stdin: Option<String>,
    pub env: BTreeMap<String, String>,
    /// OpenCode only: how many of argv's first items are the CLI itself, for the route check.
    pub prefix_len: Option<usize>,
}

pub fn turn_command(
    worker: &Worker,
    prefix: &[String],
    task: &str,
    prompt_path: &str,
    session: Option<&str>,
) -> TurnCommand {
    match worker.cli {
        Cli::Codex => {
            let (model, effort) = worker.route().split_once('#').unwrap_or((worker.route(), ""));
            let mut argv = prefix.to_vec();
            argv.extend(
                ["exec", "--json", "--dangerously-bypass-approvals-and-sandbox", "--skip-git-repo-check"]
                    .map(String::from),
            );
            if !model.is_empty() {
                argv.extend(["-m".to_string(), model.to_string()]);
            }
            if !effort.is_empty() {
                argv.extend(["-c".to_string(), format!("model_reasoning_effort={effort}")]);
            }
            if let Some(session) = session {
                argv.extend(["resume".to_string(), session.to_string()]);
            }
            argv.push("-".to_string());
            TurnCommand { argv, stdin: Some(prompt_path.to_string()), env: BTreeMap::new(), prefix_len: None }
        }
        Cli::CommandCode => {
            let (model, effort) = crate::task::commandcode::route(worker.route());
            let mut argv = prefix.to_vec();
            argv.extend(
                [
                    "-p",
                    "--output-format",
                    "json",
                    "-m",
                    &model,
                    "--effort",
                    &effort,
                    "--yolo",
                    "--skip-onboarding",
                    "-t",
                    "--no-auto-update",
                    "--max-turns",
                    "200",
                ]
                .map(String::from),
            );
            if let Some(session) = session {
                argv.extend(["--resume".to_string(), session.to_string()]);
            } else {
                argv.extend(["-n".to_string(), format!("kompitum-{task}")]);
            }
            TurnCommand {
                argv,
                stdin: Some(prompt_path.to_string()),
                env: BTreeMap::from([("COMMANDCODE_SKIP_UPDATES".to_string(), "1".to_string())]),
                prefix_len: None,
            }
        }
        Cli::Opencode => TurnCommand {
            argv: opencode_command(prefix, worker, task, prompt_path, session),
            stdin: None,
            env: BTreeMap::new(),
            prefix_len: Some(prefix.len()),
        },
        Cli::Claude => {
            let (model, effort) = worker.route().split_once('#').unwrap_or((worker.route(), ""));
            let mut argv = prefix.to_vec();
            // Never `--bare`: it reads only an API key, and the plan is a login.
            argv.extend(["-p", "--output-format", "stream-json", "--verbose", "--model", model].map(String::from));
            if !effort.is_empty() {
                argv.extend(["--effort".to_string(), effort.to_string()]);
            }
            argv.extend(
                [
                    "--permission-mode",
                    "bypassPermissions",
                    "--strict-mcp-config",
                    "--setting-sources",
                    "project,local",
                    "--disallowedTools",
                    "Agent",
                ]
                .map(String::from),
            );
            if let Some(session) = session {
                argv.extend(["--resume".to_string(), session.to_string()]);
            }
            TurnCommand { argv, stdin: Some(prompt_path.to_string()), env: BTreeMap::new(), prefix_len: None }
        }
    }
}

/// What a reader sees while a turn runs: steps, the last tools, the cost so far.
#[derive(Debug)]
pub struct Progress {
    pub steps: u64,
    pub tools: Vec<String>,
    pub cost_usd: f64,
    pub peak: u64,
    pub(crate) cli: Cli,
    /// The worktree: a tool's path inside it is shown relative to it.
    pub(crate) root: Option<PathBuf>,
    pub(crate) seen: BTreeSet<String>,
}

impl Default for Progress {
    fn default() -> Progress {
        Progress::new(Cli::Opencode, None)
    }
}

impl Progress {
    /// For a turn running in this folder.
    pub fn new(cli: Cli, root: Option<PathBuf>) -> Progress {
        Progress { steps: 0, tools: Vec::new(), cost_usd: 0.0, peak: 0, cli, root, seen: BTreeSet::new() }
    }

    /// One output line in, one line for the screen out (or nothing to show).
    pub fn feed(&mut self, line: &str) -> Option<String> {
        match self.cli {
            Cli::Codex => crate::task::codex::progress(self, line),
            Cli::CommandCode => crate::task::commandcode::progress(self, line),
            Cli::Opencode => crate::task::opencode::progress(self, line),
            Cli::Claude => crate::task::claude::progress(self, line),
        }
    }
}

pub(crate) fn text_at<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
}

pub(crate) fn part_text<'a>(part: &'a serde_json::Map<String, Value>, key: &str) -> Option<&'a str> {
    part.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
}

pub(crate) fn whole(value: Option<&Value>) -> u64 {
    value.and_then(Value::as_u64).unwrap_or(0)
}

/// The keys a tool's input names what it works on under, most telling first.
pub(crate) const TARGET_KEYS: [&str; 7] = ["filePath", "path", "file_path", "pattern", "url", "command", "query"];

/// What a tool works on, for someone watching: the first line of its input's
/// target, relative to the worktree when it lies inside. It goes on the screen
/// and in the beat, never in the task's record: a tool's input can say anything.
pub(crate) fn target_in(input: &serde_json::Map<String, Value>, root: Option<&Path>) -> Option<String> {
    let text = TARGET_KEYS.iter().find_map(|key| part_text(input, key))?;
    let first = text.lines().next().unwrap_or_default();
    let inside = root.and_then(|root| Path::new(first).strip_prefix(root).ok());
    Some(inside.map_or_else(|| first.to_string(), |path| path.display().to_string().replace('\\', "/")))
}

pub(crate) fn push_list(tools: &mut Vec<String>, shown: String) {
    tools.push(shown);
    while tools.len() > 5 {
        tools.remove(0);
    }
}

pub(crate) fn push_tool(progress: &mut Progress, shown: String) {
    push_list(&mut progress.tools, shown);
}

/// Everything a finished turn's output says, read once, away from the core thread.
#[derive(Clone, Debug, Default)]
pub struct TurnOutcome {
    /// The first session id the worker reported.
    pub session: Option<String>,
    /// Every session id it reported.
    pub sessions: Vec<String>,
    /// The last text event's text.
    pub text: String,
    /// Step-finish token totals and cost.
    pub usage: Usage,
    /// Error events, summarized as words (never the whole payload).
    pub errors: Vec<String>,
    /// The last five tools it used, oldest first.
    pub tools: Vec<String>,
    /// Non-empty lines that were not JSON objects.
    pub parse_errors: u32,
    /// Command Code's result line, when it wrote one.
    pub reported: Option<Reported>,
    /// Command Code's model_request_start models, distinct and in order.
    pub models: Vec<String>,
    /// The raw error payloads, at most five, for the quota check.
    pub raw_errors: Vec<Value>,
    /// The files the worker read with its read tool, deduped, first read first.
    pub files_read: Vec<String>,
    /// The last of the worker's reasoning, at most 8 KB.
    pub reasoning_tail: String,
}

/// How much reasoning a reader holds while the turn runs, and what it keeps.
pub(crate) const REASONING_HOLD: usize = 16 * 1024;
pub(crate) const REASONING_TAIL: usize = 8 * 1024;
/// A turn's read list stops here; anything more is not a hand-off note.
pub(crate) const FILES_READ_LIMIT: usize = 2000;

/// At most `max` bytes from the end of `text`, moved forward to a character
/// boundary so a multi-byte character is never cut.
pub(crate) fn end_bytes(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut start = text.len().saturating_sub(max);
    while start < text.len() && !text.is_char_boundary(start) {
        start += 1;
    }
    text.get(start..).unwrap_or_default().to_string()
}

/// Keep the end of a worker's reasoning, never more than `REASONING_HOLD`.
pub(crate) fn push_reasoning(tail: &mut String, text: &str) {
    if text.is_empty() {
        return;
    }
    tail.push_str(text);
    if tail.len() > REASONING_HOLD {
        *tail = end_bytes(tail, REASONING_HOLD);
    }
}

/// One file read, once, in first-read order.
pub(crate) fn push_read(files: &mut Vec<String>, path: &str) {
    if path.is_empty() || files.len() >= FILES_READ_LIMIT || files.iter().any(|seen| seen == path) {
        return;
    }
    files.push(path.to_string());
}

/// Command Code's result line: how the run says it ended.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reported {
    pub subtype: String,
    pub error: String,
    pub final_text: String,
}

impl TurnOutcome {
    pub fn from_lines(cli: Cli, lines: impl IntoIterator<Item = String>) -> TurnOutcome {
        match cli {
            Cli::Codex => crate::task::codex::read(lines),
            Cli::CommandCode => crate::task::commandcode::read(lines),
            Cli::Opencode => crate::task::opencode::read(lines),
            Cli::Claude => crate::task::claude::read(lines),
        }
    }

    pub fn from_bytes(cli: Cli, bytes: &[u8]) -> TurnOutcome {
        TurnOutcome::from_lines(cli, String::from_utf8_lossy(bytes).lines().map(String::from))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(cc: Option<&str>, oc: Option<&str>) -> Config {
        Config {
            workers: Some(crate::config::WorkersConfig {
                cap: None,
                opencode: oc.map(|p| vec![p.to_string()]),
                commandcode: cc.map(|p| vec![p.to_string()]),
                codex: None,
                claude: None,
            }),
            ..Default::default()
        }
    }

    #[test]
    fn every_cli_key_round_trips() {
        for key in ["codex", "command_code", "opencode", "claude"] {
            assert_eq!(Cli::from_key(key).map(Cli::key), Some(key));
        }
        assert_eq!(Cli::from_key("nope"), None);
    }

    #[test]
    fn a_version_is_found_wherever_it_sits_and_must_be_v2() {
        assert_eq!(first_version("v2.0.0"), Some((2, 0, 0)));
        assert_eq!(first_version("opencode 1.2.3\n"), Some((1, 2, 3)));
        assert_eq!(first_version("nothing here"), None);
        assert_eq!(first_version("path 9/2.0.0"), Some((2, 0, 0)));
    }

    fn worker(key: &str) -> Worker {
        builtin().into_iter().find(|worker| worker.key == key).expect("a built-in worker")
    }

    #[test]
    fn the_command_continues_a_session_or_names_a_new_one() {
        let prefix = vec!["opencode".to_string()];
        let go = worker("go");
        let fresh = opencode_command(&prefix, &go, "fix-1", "C:/t/prompt.md", None);
        assert_eq!(
            &fresh[..8],
            ["opencode", "run", "--standalone", "--auto", "--format", "json", "--model", go.route()]
        );
        assert!(fresh.windows(2).any(|w| w == ["--title", "kompitum-fix-1"]), "{fresh:?}");
        assert!(fresh.windows(2).any(|w| w == ["--file", "C:/t/prompt.md"]), "{fresh:?}");
        assert_eq!(
            fresh.last().unwrap(),
            "The attached file is the full task brief. Read it and carry it out exactly."
        );
        let continued = opencode_command(&prefix, &worker("free"), "fix-1", "p", Some("ses_1"));
        assert!(continued.windows(2).any(|w| w == ["--session", "ses_1"]));
        assert!(!continued.iter().any(|a| a == "--title"));
    }

    #[test]
    fn the_brief_goes_on_stdin_and_not_in_the_arguments() {
        let prefix = vec!["node".to_string(), "index.mjs".to_string()];
        let cc = worker("cc");
        let command = turn_command(&cc, &prefix, "fix-1", "C:/t/prompt.md", None);
        assert_eq!(command.stdin.as_deref(), Some("C:/t/prompt.md"));
        assert!(!command.argv.iter().any(|arg| arg.contains("prompt")), "{:?}", command.argv);
        assert_eq!(command.argv[..2], ["node", "index.mjs"]);
        assert!(command.argv.windows(2).any(|w| w == ["-n", "kompitum-fix-1"]), "{:?}", command.argv);
        assert_eq!(command.env.get("COMMANDCODE_SKIP_UPDATES").map(String::as_str), Some("1"));
        assert_eq!(command.prefix_len, None);
        let continued = turn_command(&cc, &prefix, "fix-1", "p", Some("ses_1"));
        assert!(continued.argv.windows(2).any(|w| w == ["--resume", "ses_1"]), "{:?}", continued.argv);
        assert!(!continued.argv.iter().any(|arg| arg == "-n"));
        let opencode = turn_command(&worker("go"), &["opencode".into()], "fix-1", "p", None);
        assert_eq!(opencode.stdin, None);
        assert_eq!(opencode.prefix_len, Some(1));
    }

    #[test]
    fn a_codex_turn_puts_its_flags_before_resume_and_its_brief_on_stdin() {
        let prefix = vec!["codex".to_string()];
        let codex = worker("codex");
        let fresh = turn_command(&codex, &prefix, "fix-1", "C:/t/prompt.md", None);
        assert_eq!(
            fresh.argv,
            [
                "codex",
                "exec",
                "--json",
                "--dangerously-bypass-approvals-and-sandbox",
                "--skip-git-repo-check",
                "-m",
                "gpt-6-luna",
                "-c",
                "model_reasoning_effort=high",
                "-",
            ]
        );
        assert_eq!(fresh.stdin.as_deref(), Some("C:/t/prompt.md"));
        assert_eq!(fresh.prefix_len, None);
        let continued = turn_command(&codex, &prefix, "fix-1", "p", Some("thread-1"));
        assert_eq!(&continued.argv[..9], &fresh.argv[..9], "every flag stays before resume");
        assert_eq!(&continued.argv[9..], ["resume", "thread-1", "-"]);
        assert_eq!(continued.stdin.as_deref(), Some("p"));
        assert_eq!(continued.prefix_len, None);

        for route in ["gpt-6-luna", "gpt-6-luna#"] {
            let bare = Worker { route: route.into(), ..codex.clone() };
            let bare_command = turn_command(&bare, &prefix, "fix-1", "p", None);
            assert!(!bare_command.argv.iter().any(|arg| arg == "-c"), "{route}: {:?}", bare_command.argv);
        }
        let no_model = Worker { route: "#high".into(), ..codex };
        let no_model_command = turn_command(&no_model, &prefix, "fix-1", "p", None);
        assert!(!no_model_command.argv.iter().any(|arg| arg == "-m"), "{:?}", no_model_command.argv);
    }

    #[test]
    fn the_haiku_worker_runs_claude_headless_with_its_brief_on_stdin() {
        let prefix = vec!["claude".to_string()];
        let haiku = worker("haiku");
        let fresh = turn_command(&haiku, &prefix, "fix-1", "C:/t/prompt.md", None);
        assert_eq!(
            fresh.argv,
            [
                "claude",
                "-p",
                "--output-format",
                "stream-json",
                "--verbose",
                "--model",
                "claude-haiku-5-5",
                "--effort",
                "high",
                "--permission-mode",
                "bypassPermissions",
                "--strict-mcp-config",
                "--setting-sources",
                "project,local",
                "--disallowedTools",
                "Agent",
            ]
        );
        assert_eq!(fresh.stdin.as_deref(), Some("C:/t/prompt.md"));
        assert!(!fresh.argv.iter().any(|arg| arg == "--bare"), "{:?}", fresh.argv);
        assert_eq!(fresh.prefix_len, None);
        let continued = turn_command(&haiku, &prefix, "fix-1", "p", Some("session-1"));
        assert_eq!(&continued.argv[..16], &fresh.argv[..], "every flag stays before resume");
        assert_eq!(&continued.argv[16..], ["--resume", "session-1"]);
        let bare = Worker { route: "claude-haiku-5-5".into(), ..haiku };
        let bare_command = turn_command(&bare, &prefix, "fix-1", "p", None);
        assert!(!bare_command.argv.iter().any(|arg| arg == "--effort"), "{:?}", bare_command.argv);
        assert!(bare_command.argv.windows(2).any(|w| w == ["--model", "claude-haiku-5-5"]));
    }

    #[test]
    fn an_added_worker_runs_its_own_route() {
        let opencode = Worker {
            key: "mine".into(),
            label: "Mine".into(),
            cli: Cli::Opencode,
            route: "opencode-go/other-model#max".into(),
        };
        let command = turn_command(&opencode, &["opencode".into()], "fix-1", "p", None);
        assert!(command.argv.windows(2).any(|w| w == ["--model", "opencode-go/other-model#max"]), "{:?}", command.argv);
        assert_eq!(command.prefix_len, Some(1));
        let command_code = Worker {
            key: "mine-cc".into(),
            label: "Mine CC".into(),
            cli: Cli::CommandCode,
            route: "commandcode/vendor/model#high".into(),
        };
        let command = turn_command(&command_code, &["cc".into()], "fix-1", "p", None);
        assert!(command.argv.windows(2).any(|w| w == ["-m", "vendor/model"]), "{:?}", command.argv);
        assert!(command.argv.windows(2).any(|w| w == ["--effort", "high"]), "{:?}", command.argv);
        assert_eq!(command.stdin.as_deref(), Some("p"));
    }

    #[test]
    fn a_program_that_does_not_exist_is_not_installed() {
        let missing = std::env::temp_dir().join("kompitum-no-such-worker-2419").display().to_string();
        let error = prefix(&worker("cc"), &config(Some(&missing), None)).unwrap_err();
        assert_eq!(error.kind, "worker_missing");
        assert!(error.message.contains("Command Code"), "{}", error.message);
        assert!(error.hint.contains("[workers] commandcode"), "{}", error.hint);
        let error = prefix(&worker("go"), &config(None, Some(&missing))).unwrap_err();
        assert_eq!(error.kind, "worker_missing");
        assert!(error.hint.contains("[workers] opencode"), "{}", error.hint);
        let shim = std::env::temp_dir().join("kompitum-worker-2419.cmd").display().to_string();
        let error = prefix(&worker("cc"), &config(Some(&shim), None)).unwrap_err();
        assert!(error.message.contains("shell shim"), "{}", error.message);
    }

    #[test]
    fn the_command_code_route_splits_into_a_model_and_an_effort() {
        assert_eq!(
            crate::task::commandcode::route("commandcode/deepseek/deepseek-v4.1-flash#max"),
            ("deepseek/deepseek-v4.1-flash".into(), "max".into())
        );
        assert_eq!(
            crate::task::commandcode::route("commandcode/deepseek/deepseek-v4.1-flash"),
            ("deepseek/deepseek-v4.1-flash".into(), "max".into())
        );
    }
}
