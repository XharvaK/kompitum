//! The only place that knows a worker CLI by name: how to find it, how to ask
//! its version, how to tell it to run a turn, and how to read its output.

use crate::config::Config;
use crate::rpc::RpcError;
use crate::task::types::Usage;
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Worker {
    Go,
    Free,
}

/// The label for a stored worker key; an unknown key stays as it is.
pub fn label_of(key: &str) -> String {
    Worker::from_key(key).map_or_else(|_| key.to_string(), |worker| worker.label().to_string())
}

impl Worker {
    pub fn key(self) -> &'static str {
        match self {
            Worker::Go => "go",
            Worker::Free => "free",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Worker::Go => "opencode Go",
            Worker::Free => "opencode free",
        }
    }

    pub fn route(self) -> &'static str {
        match self {
            Worker::Go => "opencode-go/deepseek-v4.1-flash#max",
            Worker::Free => "opencode/deepseek-v4.1-flash-free#max",
        }
    }

    pub fn from_key(key: &str) -> Result<Worker, RpcError> {
        match key {
            "go" => Ok(Worker::Go),
            "free" => Ok(Worker::Free),
            other => Err(RpcError::invalid_params(format!("unknown worker {other:?}: use go or free"))),
        }
    }
}

fn worker_missing(message: impl Into<String>) -> RpcError {
    RpcError::new(
        "worker_missing",
        message,
        "install OpenCode v2, or set [workers] opencode in ~/.config/limen-core/config.toml",
    )
}

/// A shell shim can't be spawned without a shell, and the real executable's
/// output is what the loop reads: refuse it by name, as the Python does.
fn check_shim(exe: &str) -> Result<(), RpcError> {
    let lower = exe.to_ascii_lowercase();
    if [".cmd", ".bat", ".ps1"].iter().any(|ext| lower.ends_with(ext)) {
        return Err(worker_missing(format!("{exe:?} is a shell shim: give the opencode executable itself")));
    }
    Ok(())
}

/// A program's file name on this system. On Windows only the `.exe` starts
/// without a shell: npm leaves a shell script called `opencode` beside it.
fn program(name: &str) -> String {
    format!("{name}{}", std::env::consts::EXE_SUFFIX)
}

fn on_path(names: &[String]) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
        .find(|candidate| candidate.is_file())
}

fn python() -> Option<Vec<String>> {
    on_path(&[program("python3"), program("python")]).map(|path| vec![path.display().to_string()])
}

/// `opencode`'s argv prefix: `[workers] opencode` when the config gives one,
/// else the executable on PATH (or the npm fallback on Windows), as
/// `command_prefix` does in the Python.
pub fn opencode_prefix(config: &Config) -> Result<Vec<String>, RpcError> {
    if let Some(mut argv) = config.opencode() {
        check_shim(&argv[0])?;
        if argv[0].to_ascii_lowercase().ends_with(".py") {
            let mut python = python().ok_or_else(|| worker_missing("no python to run OpenCode's script"))?;
            python.append(&mut argv);
            return Ok(python);
        }
        return Ok(argv);
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
        None => Err(worker_missing("OpenCode was not found on PATH")),
    }
}

/// Prefixes whose version this core already accepted: asking once is enough.
static CHECKED: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

fn first_version(text: &str) -> Option<(u32, u32, u32)> {
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
        Ok(Err(e)) => return Err(worker_missing(format!("cannot run {} --version: {e}", prefix[0]))),
        Err(_) => return Err(worker_missing(format!("{} --version did not answer within 30 s", prefix[0]))),
    };
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    match first_version(&text) {
        Some((2, _, _)) => {
            CHECKED.lock().unwrap_or_else(|p| p.into_inner()).insert(key);
            Ok(())
        }
        _ => {
            let found = crate::task::types::cut(text.trim(), 120);
            Err(worker_missing(format!(
                "lc requires OpenCode v2.x; found: {}",
                if found.is_empty() { "nothing".into() } else { found }
            )))
        }
    }
}

/// The whole command for one turn. `session` means continue that conversation.
pub fn opencode_command(
    prefix: &[String],
    worker: Worker,
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
        argv.extend(["--title".to_string(), format!("limen-{task}")]);
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

/// What a reader sees while a turn runs: steps, the last tools, the cost so far.
#[derive(Default, Debug)]
pub struct Progress {
    pub steps: u64,
    pub tools: Vec<String>,
    pub cost_usd: f64,
    pub peak: u64,
    /// The worktree: a tool's path inside it is shown relative to it.
    root: Option<PathBuf>,
    seen: BTreeSet<String>,
}

fn text_at<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
}

fn part_text<'a>(part: &'a serde_json::Map<String, Value>, key: &str) -> Option<&'a str> {
    part.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
}

fn whole(value: Option<&Value>) -> u64 {
    value.and_then(Value::as_u64).unwrap_or(0)
}

/// One step's numbers, from its `step-finish` part.
fn step_usage(part: &serde_json::Map<String, Value>) -> Usage {
    let tokens = part.get("tokens").and_then(Value::as_object);
    let cache = tokens.and_then(|t| t.get("cache")).and_then(Value::as_object);
    let input = whole(tokens.and_then(|t| t.get("input")));
    let cache_read = whole(cache.and_then(|c| c.get("read")));
    let cache_write = whole(cache.and_then(|c| c.get("write")));
    Usage {
        steps: 1,
        input,
        output: whole(tokens.and_then(|t| t.get("output"))),
        reasoning: whole(tokens.and_then(|t| t.get("reasoning"))),
        cache_read,
        cache_write,
        largest_request: input + cache_read + cache_write,
        cost_usd: part.get("cost").and_then(Value::as_f64).unwrap_or(0.0),
    }
}

/// `summarize_tool`: name and status, never the tool's input or output.
fn summarize_tool(event: &Value) -> Option<String> {
    let part = event.get("part").and_then(Value::as_object);
    let tool = part.and_then(|p| p.get("tool")).or_else(|| event.get("tool"))?;
    let (name, status) = match tool.as_object() {
        Some(tool) => {
            (tool.get("name").or_else(|| tool.get("tool")).and_then(Value::as_str), part_text(tool, "status"))
        }
        None => {
            let state = part.and_then(|p| p.get("state")).and_then(Value::as_object);
            (tool.as_str(), state.and_then(|s| part_text(s, "status")))
        }
    };
    let name = name?;
    let line = match status {
        Some(status) => format!("{name}:{status}"),
        None => name.to_string(),
    };
    Some(crate::task::types::cut(&line, 120))
}

/// The keys a tool's input names what it works on under, most telling first.
const TARGET_KEYS: [&str; 7] = ["filePath", "path", "file_path", "pattern", "url", "command", "query"];

/// What a tool works on, for someone watching: the first line of its input's
/// target, relative to the worktree when it lies inside. It goes on the screen
/// and in the beat, never in the task's record: a tool's input can say anything.
fn tool_target(event: &Value, root: Option<&Path>) -> Option<String> {
    let input = event.get("part")?.get("state")?.get("input")?.as_object()?;
    let text = TARGET_KEYS.iter().find_map(|key| part_text(input, key))?;
    let first = text.lines().next().unwrap_or_default();
    let inside = root.and_then(|root| Path::new(first).strip_prefix(root).ok());
    Some(inside.map_or_else(|| first.to_string(), |path| path.display().to_string().replace('\\', "/")))
}

impl Progress {
    /// For a turn running in this folder.
    pub fn in_folder(root: Option<PathBuf>) -> Progress {
        Progress { root, ..Default::default() }
    }

    /// One output line in, one line for the screen out (or nothing to show).
    pub fn feed(&mut self, line: &str) -> Option<String> {
        let value: Value = serde_json::from_str(line.trim()).ok()?;
        let event = value.as_object()?;
        let part = event.get("part").and_then(Value::as_object);
        let part_type = part.and_then(|p| part_text(p, "type"));
        if part_type == Some("step-finish") {
            let step = step_usage(part?);
            self.steps += 1;
            self.peak = self.peak.max(step.largest_request);
            self.cost_usd += step.cost_usd;
            return Some(format!(
                "step {}: {} in, {} out, ${:.4}",
                self.steps,
                step.input + step.cache_read,
                step.output + step.reasoning,
                self.cost_usd
            ));
        }
        if text_at(&value, "type") == Some("tool_use") || part_type == Some("tool") {
            let part = part?;
            let id = part_text(part, "partID").or_else(|| part_text(part, "id"));
            if let Some(id) = id
                && !self.seen.insert(id.to_string())
            {
                return None;
            }
            let tool = summarize_tool(&value)?;
            // "completed" on nearly every line says nothing to someone watching.
            let tool = tool.strip_suffix(":completed").unwrap_or(&tool);
            let shown = match tool_target(&value, self.root.as_deref()) {
                Some(target) => crate::task::types::cut(&format!("{tool} {target}"), 120),
                None => tool.to_string(),
            };
            self.tools.push(shown.clone());
            while self.tools.len() > 5 {
                self.tools.remove(0);
            }
            return Some(shown);
        }
        None
    }
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
}

fn summarize_error(raw: &Value) -> String {
    let Some(object) = raw.as_object() else { return "error".into() };
    let data = object.get("data").and_then(Value::as_object);
    let name = object
        .get("name")
        .or_else(|| object.get("type"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or("error");
    let code = data
        .and_then(|d| d.get("code"))
        .or_else(|| object.get("code"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty());
    let message = data
        .and_then(|d| d.get("message"))
        .or_else(|| object.get("message"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty());
    let mut out = name.to_string();
    if let Some(code) = code {
        out.push_str(&format!(" ({code})"));
    }
    if let Some(message) = message {
        out.push_str(&format!(": {message}"));
    }
    crate::task::types::cut(&out, 300)
}

impl TurnOutcome {
    pub fn from_lines(lines: impl IntoIterator<Item = String>) -> TurnOutcome {
        let mut out = TurnOutcome::default();
        for line in lines {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let Ok(value) = serde_json::from_str::<Value>(line) else {
                out.parse_errors += 1;
                continue;
            };
            let Some(event) = value.as_object() else {
                out.parse_errors += 1;
                continue;
            };
            let mut candidates = vec![event.get("sessionID"), event.get("session_id")];
            if let Some(part) = event.get("part") {
                candidates.push(part.get("sessionID"));
            }
            if let Some(properties) = event.get("properties") {
                candidates.push(properties.get("sessionID"));
            }
            for id in candidates.into_iter().flatten().filter_map(Value::as_str).filter(|s| !s.is_empty()) {
                if !out.sessions.iter().any(|seen| seen == id) {
                    out.sessions.push(id.to_string());
                }
                if out.session.is_none() {
                    out.session = Some(id.to_string());
                }
            }
            let kind = text_at(&value, "type").unwrap_or("?");
            let part = event.get("part").and_then(Value::as_object);
            if kind == "text"
                && let Some(text) = part.and_then(|p| p.get("text")).and_then(Value::as_str)
            {
                out.text = text.to_string();
            }
            if let Some(part) = part.filter(|p| part_text(p, "type") == Some("step-finish")) {
                let step = step_usage(part);
                out.usage.steps += step.steps;
                out.usage.input += step.input;
                out.usage.output += step.output;
                out.usage.reasoning += step.reasoning;
                out.usage.cache_read += step.cache_read;
                out.usage.cache_write += step.cache_write;
                out.usage.largest_request = out.usage.largest_request.max(step.largest_request);
                out.usage.cost_usd += step.cost_usd;
            }
            if kind == "error" {
                out.errors.push(summarize_error(event.get("error").unwrap_or(&value)));
            }
            if (kind == "tool_use" || part.and_then(|p| part_text(p, "type")) == Some("tool"))
                && let Some(tool) = summarize_tool(&value)
            {
                out.tools.push(tool);
                while out.tools.len() > 5 {
                    out.tools.remove(0);
                }
            }
        }
        out
    }

    pub fn from_bytes(bytes: &[u8]) -> TurnOutcome {
        TurnOutcome::from_lines(String::from_utf8_lossy(bytes).lines().map(String::from))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SESSION: &str = "ses_5glfJCtnAmRo1vsz02KjZlOl0B";

    fn line(text: &str) -> String {
        text.to_string()
    }

    fn stream() -> Vec<String> {
        vec![
            line(&format!(
                r#"{{"type":"step_start","timestamp":1793798531666,"sessionID":"{SESSION}","part":{{"id":"prt_1","type":"step-start"}}}}"#
            )),
            line(&format!(
                r#"{{"type":"text","timestamp":1793798532015,"sessionID":"{SESSION}","part":{{"id":"prt_2","type":"text","text":"I'll start by reading the task brief."}}}}"#
            )),
            line(&format!(
                r#"{{"type":"tool_use","timestamp":1793798532046,"sessionID":"{SESSION}","part":{{"partID":"prt_3","type":"tool","id":"call_1","tool":"read","state":{{"status":"completed","input":{{"path":"src/lib.rs"}},"output":"Read file"}}}}}}"#
            )),
            line(&format!(
                r#"{{"type":"step_finish","timestamp":1793798532155,"sessionID":"{SESSION}","part":{{"id":"prt_4","type":"step-finish","cost":0.001723308,"tokens":{{"input":11142,"output":79,"reasoning":2,"cache":{{"read":1536,"write":8}}}}}}}}"#
            )),
            line(&format!(
                r#"{{"type":"text","timestamp":1793798532300,"sessionID":"{SESSION}","part":{{"id":"prt_5","type":"text","text":"All done."}}}}"#
            )),
        ]
    }

    #[test]
    fn a_real_stream_gives_its_session_text_usage_and_no_errors() {
        let outcome = TurnOutcome::from_lines(stream());
        assert_eq!(outcome.session.as_deref(), Some(SESSION));
        assert_eq!(outcome.sessions, [SESSION]);
        assert_eq!(outcome.text, "All done.");
        assert_eq!(outcome.usage.steps, 1);
        assert_eq!(outcome.usage.input, 11142);
        assert_eq!(outcome.usage.output, 79);
        assert_eq!(outcome.usage.reasoning, 2);
        assert_eq!(outcome.usage.cache_read, 1536);
        assert_eq!(outcome.usage.cache_write, 8);
        assert_eq!(outcome.usage.largest_request, 11142 + 1536 + 8);
        assert!((outcome.usage.cost_usd - 0.001723308).abs() < 1e-12);
        assert!(outcome.errors.is_empty() && outcome.parse_errors == 0);
    }

    #[test]
    fn error_events_and_unparseable_lines_are_kept_as_their_own_facts() {
        let lines = vec![
            line(
                r#"{"type":"error","sessionID":"s1","error":{"name":"ProviderError","data":{"code":"429","message":"quota"}}}"#,
            ),
            line("not json"),
            line("[1,2]"),
            line(""),
        ];
        let outcome = TurnOutcome::from_lines(lines);
        assert_eq!(outcome.errors, ["ProviderError (429): quota"]);
        assert_eq!(outcome.parse_errors, 2);
        assert_eq!(outcome.session.as_deref(), Some("s1"));
        assert_eq!(outcome.text, "");
    }

    #[test]
    fn progress_counts_steps_once_and_keeps_the_last_five_tools() {
        let mut progress = Progress::default();
        let feed: Vec<_> = stream().iter().filter_map(|l| progress.feed(l)).collect();
        assert_eq!(progress.steps, 1);
        assert_eq!(progress.peak, 11142 + 1536 + 8);
        assert!((progress.cost_usd - 0.001723308).abs() < 1e-12);
        assert_eq!(feed, ["read src/lib.rs", "step 1: 12678 in, 81 out, $0.0017"]);
        // The same part twice is one tool.
        assert_eq!(progress.feed(&stream()[2]), None);
        for n in 0..6 {
            let tool = format!(
                r#"{{"type":"tool_use","part":{{"partID":"p{n}","type":"tool","tool":"bash","state":{{"status":"running"}}}}}}"#
            );
            assert_eq!(progress.feed(&tool).as_deref(), Some("bash:running"));
        }
        assert_eq!(progress.tools.len(), 5);
        assert_eq!(progress.tools[0], "bash:running");
    }

    #[test]
    fn a_tool_line_says_what_the_tool_works_on_relative_to_the_worktree() {
        let root = std::env::temp_dir().join("worktree");
        let inside = root.join("src").join("lib.rs").display().to_string();
        let mut progress = Progress::in_folder(Some(root));
        let mut feed = |tool: &str, status: &str, input: Value| {
            let id = format!("{tool}-{status}");
            let event = serde_json::json!({"type": "tool_use", "part": {
                "partID": id, "type": "tool", "tool": tool, "state": {"status": status, "input": input}}});
            progress.feed(&event.to_string())
        };
        let read = feed("read", "completed", serde_json::json!({"path": inside}));
        assert_eq!(read.as_deref(), Some("read src/lib.rs"));
        let bash = feed("bash", "running", serde_json::json!({"command": "cargo test\nnext line"}));
        assert_eq!(bash.as_deref(), Some("bash:running cargo test"));
        let outside = feed("grep", "error", serde_json::json!({"pattern": "x"}));
        assert_eq!(outside.as_deref(), Some("grep:error x"));
        let nothing = feed("list", "completed", serde_json::json!({}));
        assert_eq!(nothing.as_deref(), Some("list"));
    }

    #[test]
    fn the_command_continues_a_session_or_names_a_new_one() {
        let prefix = vec!["opencode".to_string()];
        let fresh = opencode_command(&prefix, Worker::Go, "fix-1", "C:/t/prompt.md", None);
        assert_eq!(
            &fresh[..8],
            ["opencode", "run", "--standalone", "--auto", "--format", "json", "--model", Worker::Go.route()]
        );
        assert!(fresh.windows(2).any(|w| w == ["--title", "limen-fix-1"]), "{fresh:?}");
        assert!(fresh.windows(2).any(|w| w == ["--file", "C:/t/prompt.md"]), "{fresh:?}");
        assert_eq!(
            fresh.last().unwrap(),
            "The attached file is the full task brief. Read it and carry it out exactly."
        );
        let continued = opencode_command(&prefix, Worker::Free, "fix-1", "p", Some("ses_1"));
        assert!(continued.windows(2).any(|w| w == ["--session", "ses_1"]));
        assert!(!continued.iter().any(|a| a == "--title"));
    }

    #[test]
    fn a_version_is_found_wherever_it_sits_and_must_be_v2() {
        assert_eq!(first_version("v2.0.0"), Some((2, 0, 0)));
        assert_eq!(first_version("opencode 1.2.3\n"), Some((1, 2, 3)));
        assert_eq!(first_version("nothing here"), None);
        assert_eq!(first_version("path 9/2.0.0"), Some((2, 0, 0)));
    }
}
