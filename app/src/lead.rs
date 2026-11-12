//! The lead: one conversation per CLI (`audit/REDESIGN.md`, "One stream"),
//! the Claude settings file that keeps the app's copy of its session, and the
//! argv it starts with. Decisions live here, away from the window and the
//! process, so they test on their own.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

#[path = "lead/conversation.rs"]
pub mod conversation;
mod upkeep;
pub use upkeep::*;
pub use conversation::{
    Conversation, Event, Item, Reveal, TerminalVis, event_from_hook, fold, is_compact_command,
    items_from_transcript, reveal_after_auth_status,
};

/// The compaction window each Claude terminal the app starts gets
/// (the Owner, 2026-11-08). Part 5b's settings file uses it too.
pub const AUTO_COMPACT_WINDOW: u64 = 444_000;

/// One program's entry in `programs.json`, as the lead needs it.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct CatalogEntry {
    #[serde(default)]
    pub run: Vec<String>,
    #[serde(default)]
    pub flags: Option<Flags>,
    #[serde(default)]
    pub session: Option<Sessions>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct Flags {
    #[serde(default)]
    pub model: Vec<String>,
    #[serde(default)]
    pub effort: Vec<String>,
}

/// Up to three argv templates: a new conversation, one resumed by id, and the
/// latest one in the folder for a CLI that takes no id up front.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct Sessions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume: Option<Vec<String>>,
    #[serde(default, rename = "continue", skip_serializing_if = "Option::is_none")]
    pub continuation: Option<Vec<String>>,
}

/// What `lead.streams` holds for one program: the folder its conversation is
/// in, its id where the CLI takes one, and the transcript file that holds it
/// (from SessionStart), so a restarted app can read the conversation back.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct Stream {
    pub folder: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Choice {
    Resume { id: String },
    Continue,
    New { id: String },
    Plain,
}

impl Choice {
    pub fn is_resume_or_continue(&self) -> bool {
        matches!(self, Self::Resume { .. } | Self::Continue)
    }
}

/// The record for `program`, when it has one.
pub fn stream_of(streams: &Value, program: &str) -> Option<Stream> {
    serde_json::from_value(streams.get(program)?.clone()).ok()
}

/// Which start this program gets, checked in the order the brief names:
/// resume the same folder's conversation, continue it, start a new one, else
/// a plain start.
pub fn choose(
    catalog: &CatalogEntry,
    stream: Option<&Stream>,
    folder: &str,
    fresh: &str,
) -> Choice {
    if let Some(stream) = stream.filter(|s| s.folder == folder) {
        let sessions = catalog.session.as_ref();
        if let (Some(id), Some(_)) = (
            stream.session.as_deref().filter(|id| !id.is_empty()),
            sessions.and_then(|s| s.resume.as_ref()),
        ) {
            return Choice::Resume { id: id.to_owned() };
        }
        if sessions.and_then(|s| s.continuation.as_ref()).is_some() {
            return Choice::Continue;
        }
    }
    if catalog
        .session
        .as_ref()
        .and_then(|s| s.new.as_ref())
        .is_some()
    {
        return Choice::New {
            id: fresh.to_owned(),
        };
    }
    Choice::Plain
}

/// The record to write before this start.
pub fn stream_after_choice(choice: &Choice, folder: &str, stream: Option<&Stream>) -> Stream {
    match choice {
        Choice::New { id } => Stream {
            folder: folder.to_owned(),
            session: Some(id.clone()),
            transcript: None,
        },
        Choice::Plain => Stream {
            folder: folder.to_owned(),
            session: None,
            transcript: None,
        },
        // A resumed conversation keeps its transcript until its SessionStart names one.
        Choice::Resume { .. } | Choice::Continue => Stream {
            folder: folder.to_owned(),
            session: stream.and_then(|s| s.session.clone()),
            transcript: stream.and_then(|s| s.transcript.clone()),
        },
    }
}

/// The argv a start runs: the program, the session template for this choice,
/// the rest of `run`, then the model and reasoning flags.
pub fn argv(
    catalog: &CatalogEntry,
    choice: &Choice,
    model: Option<&str>,
    effort: Option<&str>,
) -> Vec<String> {
    let Some(first) = catalog.run.first() else {
        return Vec::new();
    };
    let mut out = vec![first.clone()];
    let template = match choice {
        Choice::Resume { .. } => catalog.session.as_ref().and_then(|s| s.resume.as_ref()),
        Choice::Continue => catalog
            .session
            .as_ref()
            .and_then(|s| s.continuation.as_ref()),
        Choice::New { .. } => catalog.session.as_ref().and_then(|s| s.new.as_ref()),
        Choice::Plain => None,
    };
    if let Some(template) = template {
        let id = match choice {
            Choice::Resume { id } | Choice::New { id } => id.as_str(),
            _ => "",
        };
        out.extend(template.iter().map(|word| word.replace("{id}", id)));
    }
    out.extend(catalog.run.iter().skip(1).cloned());
    if let Some(model) = model.filter(|m| !m.is_empty())
        && let Some(flags) = &catalog.flags
    {
        out.extend(flags.model.iter().map(|word| {
            word.replace("{model}", model)
                .replace("{effort}", effort.unwrap_or(""))
        }));
    }
    if let Some(effort) = effort.filter(|e| !e.is_empty())
        && let Some(flags) = &catalog.flags
    {
        out.extend(flags.effort.iter().map(|word| {
            word.replace("{effort}", effort)
                .replace("{model}", model.unwrap_or(""))
        }));
    }
    out
}

/// A terminal is the lead when it is live and carries the lead label with the
/// lead's program key: a terminal that ended is not the lead, so `lead.open`
/// starts a new one. Its program isn't checked: on Windows the core sees the
/// lead's wrapper shell, and that wrapper ends with the lead.
pub fn is_lead_record(record: &Value, program: &str) -> bool {
    record.get("state").and_then(Value::as_str) == Some("live")
        && record.pointer("/labels/lead").and_then(Value::as_str) == Some(program)
}

/// Nanoseconds since the Unix epoch, the clock a fresh id mixes in.
pub fn now_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

/// A version 4 UUID in lower case, from 16 bytes of SHA-256 over the time,
/// process and folder. Unique, not secret.
pub fn fresh_id(now_ns: u128, pid: u32, folder: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(now_ns.to_le_bytes());
    digest.update(pid.to_le_bytes());
    digest.update(folder.as_bytes());
    let bytes = digest.finalize();
    let mut id = [0u8; 16];
    id.copy_from_slice(&bytes[..16]);
    id[6] = (id[6] & 0x0f) | 0x40;
    id[8] = (id[8] & 0x3f) | 0x80;
    let hex = |part: &[u8]| part.iter().map(|b| format!("{b:02x}")).collect::<String>();
    format!(
        "{}-{}-{}-{}-{}",
        hex(&id[..4]),
        hex(&id[4..6]),
        hex(&id[6..8]),
        hex(&id[8..10]),
        hex(&id[10..])
    )
}

/// Whether a resume or continue ended within ten seconds with a non-zero exit
/// code, and so gets one fresh start.
pub fn retry_needed(record: &Value, already_retried: bool) -> bool {
    if already_retried || record.get("reason").and_then(Value::as_str) != Some("exited") {
        return false;
    }
    if record
        .get("exit_code")
        .and_then(Value::as_i64)
        .is_none_or(|code| code == 0)
    {
        return false;
    }
    let Some(elapsed) = record
        .get("finished_ms")
        .and_then(Value::as_u64)
        .zip(record.get("created_ms").and_then(Value::as_u64))
        .map(|(finished, created)| finished.saturating_sub(created))
    else {
        return false;
    };
    elapsed <= 10_000
}

/// Claude Code's `modelSettings`: the four models the app starts, each with the
/// same compaction window (the Owner, 2026-11-08).
pub fn model_settings() -> Value {
    let mut models = Map::new();
    for model in ["claude-fable-5-1", "claude-opus-5-5", "claude-sonnet-5-5", "claude-haiku-5-5"] {
        models.insert(model.into(), json!({ "autoCompactWindow": AUTO_COMPACT_WINDOW }));
    }
    models.into()
}

/// The settings file a Claude lead starts with: one memory (the vault's), the
/// compaction window for each model, and the hooks that bring Claude's prompts
/// into the composer. Each hook names the app's home: the lead's terminal is the
/// core's, and its environment needn't carry `KOMPITUM_APP_HOME`.
pub fn claude_settings(app: &Path, core: &Path, home: &Path) -> Value {
    let app = app.to_string_lossy();
    let core = core.to_string_lossy();
    let home = home.to_string_lossy();
    let command = |event: &str| {
        json!({
            "type": "command",
            "command": &*app,
            "args": ["hook", event, &*home],
        })
    };
    let blocking = |event: &str, timeout: u64| {
        let mut hook = command(event);
        hook["timeout"] = json!(timeout);
        hook
    };
    let async_hook = |event: &str| {
        let mut hook = command(event);
        hook["async"] = json!(true);
        hook
    };
    let mut hooks = Map::new();
    hooks.insert(
        "SessionStart".into(),
        json!([{
            "hooks": [
                blocking("SessionStart", 10),
                {
                    "type": "command",
                    "command": &*core,
                    "args": ["vault", "memory", "--hook"],
                    "timeout": 30,
                },
            ],
        }]),
    );
    hooks.insert(
        "PermissionRequest".into(),
        json!([{ "hooks": [blocking("PermissionRequest", 3600)] }]),
    );
    hooks.insert(
        "PreToolUse".into(),
        json!([
            {
                "matcher": "AskUserQuestion",
                "hooks": [blocking("PreToolUse", 3600)],
            },
            { "hooks": [async_hook("PreToolUse")] },
        ]),
    );
    hooks.insert(
        "PostToolUse".into(),
        json!([{ "hooks": [async_hook("PostToolUse")] }]),
    );
    hooks.insert(
        "PostToolUseFailure".into(),
        json!([{ "hooks": [async_hook("PostToolUseFailure")] }]),
    );
    let mut display = async_hook("MessageDisplay");
    display["timeout"] = json!(10);
    hooks.insert("MessageDisplay".into(), json!([{ "hooks": [display] }]));
    for event in ["UserPromptSubmit", "Stop", "StopFailure", "Elicitation"] {
        hooks.insert(event.into(), json!([{ "hooks": [async_hook(event)] }]));
    }
    let notifications: Vec<Value> = [
        "idle_prompt",
        "elicitation_dialog",
        "elicitation_url_dialog",
        "agent_needs_input",
        "quota_auto_resume_stale",
        "auth_success",
        "auth_storage_failure",
    ]
    .into_iter()
    .map(|matcher| {
        json!({
            "matcher": matcher,
            "hooks": [async_hook("Notification")],
        })
    })
    .collect();
    hooks.insert("Notification".into(), Value::Array(notifications));
    hooks.insert("PreCompact".into(), json!([{ "hooks": [blocking("PreCompact", 10)] }]));
    hooks.insert("PostCompact".into(), json!([{ "hooks": [async_hook("PostCompact")] }]));
    json!({
        "autoMemoryEnabled": false,
        "modelSettings": model_settings(),
        "statusLine": { "type": "command", "command": status_line_command(&app, &home) },
        "hooks": hooks,
    })
}

/// One path as a word in a shell line Claude Code runs under Git Bash: forward slashes (a backslash
/// is an escape there), and double quotes when the path has a space.
pub fn shell_word(path: &str) -> String {
    let path = path.replace('\\', "/");
    if path.contains(' ') { format!("\"{path}\"") } else { path }
}

/// The lead's status line command: the app's exe, `hook StatusLine`, then the app home.
pub fn status_line_command(app: &str, home: &str) -> String {
    format!("{} hook StatusLine {}", shell_word(app), shell_word(home))
}

/// What the lead's PreCompact hook hands Claude as the instructions for its summary.
pub const COMPACT_INSTRUCTIONS: &str = "Keep every decision the Owner made and the Owner's own words for it, what is still open or unfinished, the names of the files, tasks and people involved, and what was promised next. Drop tool output and anything already finished and merged.";

/// Writes the lead's Claude settings file, made fresh every start.
pub fn write_claude_settings(app_dir: &Path, app: &Path, core: &Path) -> Result<PathBuf, String> {
    let path = app_dir.join("lead").join("claude-settings.json");
    let fail = |error: std::io::Error| format!("Couldn't write the lead's settings file {}: {error}", path.display());
    let text = serde_json::to_string_pretty(&claude_settings(app, core, app_dir)).unwrap_or_default() + "\n";
    std::fs::create_dir_all(path.parent().unwrap_or(app_dir))
        .and_then(|()| crate::persist::write_atomic(&path, text))
        .map_err(fail)?;
    Ok(path)
}

/// The kompitum-core skill, installed into Claude Code by `lead.setup`.
pub const SKILL: &str = include_str!("../../skills/kompitum-core/SKILL.md");

/// The two `claude mcp` argvs: asking whether the tools are there, and adding
/// them for every project with this core's path.
pub fn mcp_argvs(kom: &Path) -> (Vec<String>, Vec<String>) {
    let get = vec!["mcp".into(), "get".into(), "kompitum".into()];
    let add = vec![
        "mcp".into(),
        "add".into(),
        "--scope".into(),
        "user".into(),
        "kompitum".into(),
        "--".into(),
        kom.to_string_lossy().into_owned(),
        "mcp".into(),
    ];
    (get, add)
}

/// Runs one `claude` command through the core's `exec`, so a CLI an installer
/// has just put on PATH is found: its exit code, stdout and stderr.
fn claude_exec(kom: &Path, args: &[String]) -> Result<(i32, String, String), String> {
    let mut argv = vec!["exec".into(), "--".into(), "claude".into()];
    argv.extend(args.iter().cloned());
    crate::mods::run_blocking(
        &kom.to_string_lossy(),
        &argv,
        None,
        crate::mods::EXEC_LIMIT,
    )
}

fn first_line(stderr: &str, stdout: &str, code: i32) -> String {
    stderr
        .lines()
        .chain(stdout.lines())
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| format!("exit {code}"))
}

/// Writes the kom skill below the person's home, only when it isn't there.
fn install_skill(home: &Path, skill: &str) -> Result<&'static str, String> {
    let path = home
        .join(".claude")
        .join("skills")
        .join("kompitum-core")
        .join("SKILL.md");
    if std::fs::symlink_metadata(&path).is_ok() {
        return Ok("present");
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            format!(
                "Couldn't install Kompitum's skill at {}: {error}",
                path.display()
            )
        })?;
    }
    crate::persist::write_atomic(&path, skill).map_err(|error| {
        format!(
            "Couldn't install Kompitum's skill at {}: {error}",
            path.display()
        )
    })?;
    Ok("installed")
}

/// `lead.setup`: Kompitum's MCP tools and skill for the Claude Code lead.
/// It changes nothing of Claude's own settings or login.
pub fn setup(kom: &Path, home: &Path) -> Result<Value, String> {
    let (get, add) = mcp_argvs(kom);
    let mcp = match claude_exec(kom, &get) {
        Ok((0, _, _)) => "present",
        _ => match claude_exec(kom, &add) {
            Ok((0, _, _)) => "added",
            Ok((code, out, err)) => {
                return Err(format!(
                    "Couldn't add Kompitum's tools to Claude Code: {}",
                    first_line(&err, &out, code)
                ));
            }
            Err(error) => {
                return Err(format!(
                    "Couldn't add Kompitum's tools to Claude Code: {error}"
                ));
            }
        },
    };
    let skill = install_skill(home, SKILL)?;
    Ok(json!({ "mcp": mcp, "skill": skill }))
}

/// Where every Claude Code the app starts on this machine reads its compaction
/// window: `<app folder>/claude-settings.json`.
pub fn app_claude_settings_path(app_dir: &Path) -> PathBuf {
    app_dir.join("claude-settings.json")
}

/// Writes the settings file every Claude Code the app starts gets: only
/// `modelSettings`, the 444K compaction window.
pub fn write_app_claude_settings(app_dir: &Path) -> Result<PathBuf, String> {
    let path = app_claude_settings_path(app_dir);
    let fail =
        |error: std::io::Error| format!("Couldn't write Claude Code's settings file {}: {error}", path.display());
    let text = serde_json::to_string_pretty(&json!({ "modelSettings": model_settings() })).unwrap_or_default() + "\n";
    std::fs::create_dir_all(app_dir).and_then(|()| crate::persist::write_atomic(&path, text)).map_err(fail)?;
    Ok(path)
}

/// Whether a new terminal's argv is a local Claude Code start without its own
/// `--settings`.
pub fn needs_claude_settings(argv: &[String], on: Option<&str>) -> bool {
    on.is_none()
        && argv.first().is_some_and(|program| is_claude(program))
        && !argv.iter().any(|word| word == "--settings")
}

fn is_claude(program: &str) -> bool {
    let path = Path::new(program);
    if path.file_stem().and_then(|stem| stem.to_str()).is_none_or(|stem| !stem.eq_ignore_ascii_case("claude")) {
        return false;
    }
    match path.extension().and_then(|ext| ext.to_str()) {
        None => true,
        Some(ext) => ext.eq_ignore_ascii_case("exe") || ext.eq_ignore_ascii_case("cmd"),
    }
}

/// Inserts `--settings <file>` right after the program when this is a local
/// Claude Code start without one; everything else stays as it is.
pub fn with_claude_settings(argv: Vec<String>, on: Option<&str>, file: &Path) -> Vec<String> {
    if !needs_claude_settings(&argv, on) {
        return argv;
    }
    let mut out = Vec::with_capacity(argv.len() + 2);
    out.push(argv[0].clone());
    out.push("--settings".into());
    out.push(file.to_string_lossy().into_owned());
    out.extend(argv.into_iter().skip(1));
    out
}

/// What the composer shows for one lead terminal.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Working,
    Asking,
    Waiting,
}

/// What a blocking hook is asking.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AskKind {
    Permission,
    Plan,
    Question,
}

/// Claude's open ask, as the app holds it and the composer reads it.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Ask {
    pub id: String,
    pub terminal: String,
    pub kind: AskKind,
    pub input: Value,
}

/// One terminal's lead state and ask.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct Session {
    #[serde(default)]
    pub state: Option<State>,
    #[serde(default)]
    pub ask: Option<Ask>,
    /// The conversation the panel draws, and whether the terminal is shown.
    #[serde(default)]
    pub conversation: Conversation,
    /// Whether this window has read the lead's transcript into the conversation (at a restart or a
    /// SessionStart). Not saved: a new window reads it again once.
    #[serde(skip)]
    pub history_read: bool,
    /// Whether this window has seen the lead's SessionStart. Not saved.
    #[serde(skip)]
    pub started: bool,
    /// The model Claude says it runs (`{ id, name }`), from its status line. Not saved.
    #[serde(skip)]
    pub model: Option<Value>,
    /// How full the context is and Claude's limits, from its status line. Not saved.
    #[serde(skip)]
    pub usage: Option<Value>,
    /// Whether Claude is compacting now: PreCompact sets it, PostCompact and SessionStart clear it. Not saved.
    #[serde(skip)]
    pub compacting: bool,
    /// When the state last became Waiting (ms since the Unix epoch). Not saved.
    #[serde(skip)]
    pub waiting_since_ms: Option<u64>,
    /// When `compacting` last became true (ms since the Unix epoch). Not saved.
    #[serde(skip)]
    pub compacting_since_ms: Option<u64>,
    /// The app has sent `/compact` by itself in this stretch of waiting. Not saved.
    #[serde(skip)]
    pub auto_compact_tried: bool,
}

impl Session {
    /// Sets the state and returns whether it changed. Waiting starts its clock, anything else stops
    /// it, and Working lets the app compact again in the next stretch of waiting.
    pub fn set_state(&mut self, state: Option<State>, now_ms: u64) -> bool {
        if self.state == state {
            return false;
        }
        self.state = state;
        self.waiting_since_ms = (state == Some(State::Waiting)).then_some(now_ms);
        if state == Some(State::Working) {
            self.auto_compact_tried = false;
        }
        true
    }

    /// Sets whether Claude is compacting; the clock starts when it first becomes true.
    pub fn set_compacting(&mut self, on: bool, now_ms: u64) {
        if !on {
            self.compacting_since_ms = None;
        } else if !self.compacting || self.compacting_since_ms.is_none() {
            self.compacting_since_ms = Some(now_ms);
        }
        self.compacting = on;
    }
}

/// The kind of ask a blocking hook makes, or None for an event that is not one.
pub fn ask_kind(event: &str, input: &Value) -> Option<AskKind> {
    let tool = input.get("tool_name").and_then(Value::as_str).unwrap_or_default();
    match event {
        "PermissionRequest" => Some(if tool == "ExitPlanMode" { AskKind::Plan } else { AskKind::Permission }),
        "PreToolUse" if tool == "AskUserQuestion" => Some(AskKind::Question),
        _ => None,
    }
}

/// What `lead()` publishes: state, ask, reveal, and conversation meta, never the items.
pub fn published_session(session: &Session) -> Value {
    json!({
        "state": session.state,
        "ask": session.ask,
        "terminal": session.conversation.terminal,
        "reveal": session.conversation.reveal,
        "reveal_why": session.conversation.reveal_why,
        "started": session.started,
        "model": session.model,
        "usage": session.usage,
        "compacting": session.compacting,
        "conversation": {
            "version": session.conversation.version,
            "length": session.conversation.items.len(),
        },
    })
}

/// Folds a hook into the session when it is a conversation event.
pub fn apply_hook_event(session: &mut Session, event: &str, input: &Value) -> bool {
    let Some(event) = event_from_hook(event, input) else {
        return false;
    };
    session.conversation = fold(std::mem::take(&mut session.conversation), &event);
    true
}

/// Reads the SessionStart transcript into the same items `fold` builds.
/// Returns how many unknown line types were skipped. A missing path, or a session whose transcript
/// isn't written yet, is no history at all; a file that is there but can't be read is a notice.
pub fn apply_transcript_path(session: &mut Session, path: Option<&str>) -> u32 {
    let Some(path) = path.map(str::trim).filter(|path| !path.is_empty()) else {
        return 0;
    };
    let (event, skipped) = match std::fs::read_to_string(path) {
        Ok(text) => {
            let loaded = items_from_transcript(&text);
            (Event::History { items: loaded.items }, loaded.skipped)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return 0,
        Err(_) => (Event::HistoryUnread, 0),
    };
    session.conversation = fold(std::mem::take(&mut session.conversation), &event);
    skipped
}

/// A check around the lead's start failed: the panel says so in plain words. A lead that hasn't
/// reached SessionStart keeps its terminal on screen; a login or manual reveal stays as it is.
pub fn note_lead_check_failed(conversation: &mut Conversation, started: bool, text: String) {
    let id = conversation.next("notice");
    conversation.items.push(Item::Notice { id, text });
    conversation.touched();
    if !started && conversation.reveal.is_none() {
        apply_startup_reveal(conversation);
    }
}

/// Whether a Claude lead found running when this window opened is past its SessionStart.
/// `marker` is the shared "lead.started": the terminal whose SessionStart came last, or ""
/// while a lead is starting. No marker means a build from before it existed: trust the lead.
pub fn started_on_reattach(marker: Option<&str>, terminal: &str) -> bool {
    match marker {
        None => true,
        Some(marker) => marker == terminal,
    }
}

/// Starts a lead from `claude auth status`'s exit code only.
pub fn apply_auth_status(conversation: &mut Conversation, code: i32) {
    let (terminal, reveal) = reveal_after_auth_status(code);
    conversation.terminal = terminal;
    conversation.reveal = reveal;
    conversation.reveal_why = match reveal {
        Some(Reveal::Login) => Some("Claude needs you to sign in.".into()),
        _ => None,
    };
}

/// Shows the terminal while Claude asks its startup question, before the conversation begins.
pub fn apply_startup_reveal(conversation: &mut Conversation) {
    conversation.show(
        Reveal::Startup,
        Some("Claude is asking something before it starts.".into()),
    );
}

/// Hides the startup reveal only; a login or manual reveal stays.
pub fn hide_startup(conversation: &mut Conversation) {
    if conversation.reveal == Some(Reveal::Startup) {
        conversation.hide();
    }
}

/// Runs `claude auth status` through the core. Only the exit code is kept.
pub fn claude_auth_status_code(kom: &Path) -> Result<i32, String> {
    let (code, stdout, stderr) = claude_exec(kom, &["auth".into(), "status".into()])?;
    drop((stdout, stderr));
    Ok(code)
}

/// The Owner's answer to a card.
#[derive(Clone, Debug, PartialEq)]
pub enum Answer {
    Allow,
    AllowAlways,
    Deny { note: Option<String> },
    Approve,
    KeepPlanning { note: Option<String> },
    Answers(Map<String, Value>),
    Terminal,
}

impl Answer {
    /// The answer the composer sends as `lead.answer`'s `answer`.
    pub fn from_json(value: &Value) -> Option<Self> {
        let action = value.get("action").and_then(Value::as_str)?;
        let note = || {
            value.get("note").and_then(Value::as_str).map(str::trim).filter(|note| !note.is_empty()).map(str::to_owned)
        };
        match action {
            "allow" => Some(Self::Allow),
            "allow_always" => Some(Self::AllowAlways),
            "deny" => Some(Self::Deny { note: note() }),
            "approve" => Some(Self::Approve),
            "keep_planning" => Some(Self::KeepPlanning { note: note() }),
            "answers" => Some(Self::Answers(value.get("answers")?.as_object()?.clone())),
            "terminal" => Some(Self::Terminal),
            _ => None,
        }
    }
}

/// The JSON a blocking hook prints for this ask and answer; None means no
/// output, so Claude shows its own prompt.
pub fn hook_output(ask: &Ask, answer: &Answer) -> Option<Value> {
    if *answer == Answer::Terminal {
        return None;
    }
    match (&ask.kind, answer) {
        (AskKind::Permission, Answer::Allow) => Some(permission_output("allow", None, None, None)),
        (AskKind::Permission, Answer::AllowAlways) => Some(permission_output(
            "allow",
            None,
            None,
            Some(ask.input.get("permission_suggestions").cloned().unwrap_or_else(|| json!([]))),
        )),
        (AskKind::Permission, Answer::Deny { note }) => Some(permission_output(
            "deny",
            Some(note.clone().filter(|note| !note.trim().is_empty()).unwrap_or_else(|| "The Owner declined.".into())),
            None,
            None,
        )),
        (AskKind::Plan, Answer::Approve) => Some(permission_output(
            "allow",
            None,
            Some(ask.input.get("tool_input").cloned().unwrap_or_else(|| json!({}))),
            None,
        )),
        (AskKind::Plan, Answer::KeepPlanning { note }) => Some(permission_output(
            "deny",
            Some(note.clone().filter(|note| !note.trim().is_empty()).unwrap_or_default()),
            None,
            None,
        )),
        (AskKind::Question, Answer::Answers(answers)) => Some(question_output(ask, answers)),
        _ => None,
    }
}

fn permission_output(
    behavior: &str,
    message: Option<String>,
    updated_input: Option<Value>,
    updated_permissions: Option<Value>,
) -> Value {
    let mut decision = json!({ "behavior": behavior });
    if let Some(message) = message {
        decision["message"] = json!(message);
    }
    if let Some(input) = updated_input {
        decision["updatedInput"] = input;
    }
    if let Some(permissions) = updated_permissions {
        decision["updatedPermissions"] = permissions;
    }
    json!({
        "hookSpecificOutput": {
            "hookEventName": "PermissionRequest",
            "decision": decision,
        },
    })
}

fn question_output(ask: &Ask, answers: &Map<String, Value>) -> Value {
    let questions = ask.input.pointer("/tool_input/questions").cloned().unwrap_or_else(|| json!([]));
    let mut chosen = Map::new();
    for question in questions.as_array().into_iter().flatten() {
        let Some(text) = question.get("question").and_then(Value::as_str) else {
            continue;
        };
        let Some(value) = answers.get(text) else {
            continue;
        };
        let joined = match value {
            Value::Array(labels) => labels.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "),
            Value::String(label) => label.clone(),
            _ => continue,
        };
        chosen.insert(text.to_owned(), Value::String(joined));
    }
    json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "allow",
            "updatedInput": {
                "questions": questions,
                "answers": chosen,
            },
        },
    })
}

/// A SessionStart from the lead's own terminal records the id, folder and
/// transcript path the app should resume. Anything else changes nothing.
pub fn record_session_start(
    streams: &mut Value,
    from_lead: bool,
    program_is_claude: bool,
    event: &str,
    session_id: Option<&str>,
    folder: &str,
    transcript: Option<&str>,
) -> bool {
    let Some(id) = session_id.map(str::trim).filter(|id| !id.is_empty()) else {
        return false;
    };
    if !from_lead || !program_is_claude || event != "SessionStart" {
        return false;
    }
    let transcript = transcript.map(str::trim).filter(|path| !path.is_empty());
    if !streams.is_object() {
        *streams = json!({});
    }
    let all = streams.as_object_mut().expect("made an object");
    let entry = all.entry("claude").or_insert_with(|| json!({}));
    if !entry.is_object() {
        *entry = json!({});
    }
    let stream = entry.as_object_mut().expect("made an object");
    if stream.get("session").and_then(Value::as_str) == Some(id)
        && stream.get("folder").and_then(Value::as_str) == Some(folder)
        && stream.get("transcript").and_then(Value::as_str) == transcript
    {
        return false;
    }
    stream.insert("folder".into(), json!(folder));
    stream.insert("session".into(), json!(id));
    match transcript {
        Some(path) => stream.insert("transcript".into(), json!(path)),
        None => stream.remove("transcript"),
    };
    true
}

#[cfg(test)]
#[path = "lead/tests.rs"]
mod tests;
