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
/// in, and its id where the CLI takes one.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct Stream {
    pub folder: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
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
        },
        Choice::Plain => Stream {
            folder: folder.to_owned(),
            session: None,
        },
        Choice::Resume { .. } | Choice::Continue => Stream {
            folder: folder.to_owned(),
            session: stream.and_then(|s| s.session.clone()),
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
    json!({
        "autoMemoryEnabled": false,
        "modelSettings": model_settings(),
        "hooks": hooks,
    })
}

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
    /// The Owner types in the lead's terminal itself ("Type in terminal"); otherwise typing there goes to
    /// the composer.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub typing: bool,
}

/// The kind of ask a blocking hook makes, or None for an event that is not one.
pub fn ask_kind(event: &str, input: &Value) -> Option<AskKind> {
    let tool = input.get("tool_name").and_then(Value::as_str).unwrap_or_default();
    match event {
        "PermissionRequest" => Some(if tool == "ExitPlanMode" { AskKind::Plan } else { AskKind::Permission }),
        "PreToolUse" => Some(if tool == "AskUserQuestion" { AskKind::Question } else { AskKind::Permission }),
        _ => None,
    }
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

/// A SessionStart from the lead's own terminal records the id and folder the
/// app should resume. Anything else changes nothing.
pub fn record_session_start(
    streams: &mut Value,
    from_lead: bool,
    program_is_claude: bool,
    event: &str,
    session_id: Option<&str>,
    folder: &str,
) -> bool {
    let Some(id) = session_id.map(str::trim).filter(|id| !id.is_empty()) else {
        return false;
    };
    if !from_lead || !program_is_claude || event != "SessionStart" {
        return false;
    }
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
    {
        return false;
    }
    stream.insert("folder".into(), json!(folder));
    stream.insert("session".into(), json!(id));
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog(session: Value, flags: Option<Value>) -> CatalogEntry {
        serde_json::from_value(json!({
            "run": ["prog", "--rest"],
            "flags": flags,
            "session": session,
        }))
        .unwrap()
    }

    #[test]
    fn the_argv_puts_the_session_right_after_the_program() {
        let entry = catalog(
            json!({
                "new": ["--session-id", "{id}"],
                "resume": ["--resume", "{id}"],
                "continue": ["resume", "--last"],
            }),
            Some(json!({"model": ["-m", "{model}#{effort}"], "effort": []})),
        );
        assert_eq!(
            argv(&entry, &Choice::New { id: "abc".into() }, None, None),
            ["prog", "--session-id", "abc", "--rest"]
        );
        assert_eq!(
            argv(&entry, &Choice::Resume { id: "abc".into() }, None, None),
            ["prog", "--resume", "abc", "--rest"]
        );
        assert_eq!(
            argv(&entry, &Choice::Continue, None, None),
            ["prog", "resume", "--last", "--rest"]
        );
        assert_eq!(
            argv(
                &entry,
                &Choice::Continue,
                Some("opus"),
                Some("high")
            ),
            ["prog", "resume", "--last", "--rest", "-m", "opus#high"]
        );
        assert_eq!(argv(&entry, &Choice::Plain, None, None), ["prog", "--rest"]);
        assert_eq!(
            argv(
                &entry,
                &Choice::Resume { id: "abc".into() },
                Some("opus"),
                Some("high")
            ),
            ["prog", "--resume", "abc", "--rest", "-m", "opus#high"]
        );
        assert_eq!(
            argv(&entry, &Choice::Plain, Some("opus"), None),
            ["prog", "--rest", "-m", "opus#"]
        );
        let both = catalog(
            json!({"new": ["--session-id", "{id}"]}),
            Some(json!({"model": ["--model", "{model}"], "effort": ["--effort", "{effort}"]})),
        );
        assert_eq!(
            argv(
                &both,
                &Choice::New { id: "one".into() },
                Some("opus"),
                Some("high")
            ),
            [
                "prog",
                "--session-id",
                "one",
                "--rest",
                "--model",
                "opus",
                "--effort",
                "high"
            ]
        );
        assert_eq!(
            argv(&both, &Choice::New { id: "one".into() }, None, Some("high")),
            ["prog", "--session-id", "one", "--rest", "--effort", "high"]
        );
    }

    #[test]
    fn the_choice_follows_the_record_and_the_folder() {
        let entry = catalog(
            json!({
                "new": ["--session-id", "{id}"],
                "resume": ["--resume", "{id}"],
                "continue": ["resume", "--last"],
            }),
            None,
        );
        let stream = Stream {
            folder: "/home/me".into(),
            session: Some("old".into()),
        };
        assert_eq!(
            choose(&entry, Some(&stream), "/home/me", "fresh"),
            Choice::Resume { id: "old".into() }
        );
        assert_eq!(
            choose(&entry, Some(&stream), "/home/projects", "fresh"),
            Choice::New { id: "fresh".into() }
        );
        // Another program's record is not this one's.
        assert_eq!(
            choose(&entry, None, "/home/me", "fresh"),
            Choice::New { id: "fresh".into() }
        );
        let records = json!({
            "claude": {"folder": "/home/me", "session": "old"},
            "codex": {"folder": "/home/me"},
        });
        assert_eq!(
            stream_of(&records, "claude"),
            Some(Stream {
                folder: "/home/me".into(),
                session: Some("old".into())
            })
        );
        assert_eq!(
            stream_of(&records, "codex"),
            Some(Stream {
                folder: "/home/me".into(),
                session: None
            })
        );
        assert_eq!(stream_of(&records, "grok"), None);
        let by_folder = catalog(json!({"continue": ["resume", "--last"]}), None);
        assert_eq!(
            choose(&by_folder, Some(&stream), "/home/me", "fresh"),
            Choice::Continue
        );
        assert_eq!(choose(&by_folder, None, "/home/me", "fresh"), Choice::Plain);
        let plain = catalog(Value::Null, None);
        assert_eq!(
            choose(&plain, Some(&stream), "/home/me", "fresh"),
            Choice::Plain
        );
        assert_eq!(
            stream_after_choice(&Choice::New { id: "new".into() }, "/home/me", Some(&stream)),
            Stream {
                folder: "/home/me".into(),
                session: Some("new".into())
            }
        );
        assert_eq!(
            stream_after_choice(&Choice::Plain, "/home/me", Some(&stream)),
            Stream {
                folder: "/home/me".into(),
                session: None
            }
        );
        assert_eq!(
            stream_after_choice(&Choice::Continue, "/home/me", Some(&stream)),
            stream
        );
    }

    #[test]
    fn a_fresh_id_is_a_lowercase_version_4_uuid() {
        let id = fresh_id(1_700_000_000_000_000_000, 4242, "/home/me");
        assert_eq!(id.len(), 36);
        let fields: Vec<&str> = id.split('-').collect();
        assert_eq!(
            fields.iter().map(|f| f.len()).collect::<Vec<_>>(),
            [8, 4, 4, 4, 12]
        );
        assert!(id.chars().all(|c| c.is_ascii_hexdigit() || c == '-'));
        assert!(id.chars().all(|c| !c.is_ascii_uppercase()));
        assert_eq!(&id[14..15], "4");
        assert!(["8", "9", "a", "b"].contains(&&id[19..20]));
        assert_ne!(id, fresh_id(1_700_000_000_000_000_001, 4242, "/home/me"));
        assert_ne!(id, fresh_id(1_700_000_000_000_000_000, 4242, "/home/you"));
    }

    #[test]
    fn a_labelled_shell_is_not_the_lead() {
        let lead = json!({
            "state": "live",
            "labels": {"lead": "claude"},
        });
        assert!(is_lead_record(&lead, "claude"));
        assert!(!is_lead_record(&lead, "codex"));
        let ended = json!({
            "state": "exited",
            "labels": {"lead": "claude"},
        });
        assert!(
            !is_lead_record(&ended, "claude"),
            "an ended terminal is not the lead"
        );
        let lost = json!({
            "state": "live",
            "labels": {"lead": "shell"},
        });
        assert!(!is_lead_record(&lost, "claude"));
        let unlabelled = json!({"state": "live", "labels": {}});
        assert!(!is_lead_record(&unlabelled, "claude"));
        let old_label = json!({"state": "live", "labels": {"lead": "1"}});
        assert!(!is_lead_record(&old_label, "claude"));
        let in_its_wrapper = json!({
            "state": "live",
            "labels": {"lead": "claude"},
            "program": {"key": "shell", "shell": true},
        });
        assert!(
            is_lead_record(&in_its_wrapper, "claude"),
            "on Windows the core sees the lead's wrapper shell"
        );
    }

    #[test]
    fn a_failed_resume_is_retried_once() {
        let failed = json!({
            "reason": "exited", "exit_code": 7, "created_ms": 1000, "finished_ms": 9000
        });
        assert!(retry_needed(&failed, false));
        assert!(!retry_needed(&failed, true), "only one retry");
        let mut late = failed.clone();
        late["finished_ms"] = json!(12_000);
        assert!(!retry_needed(&late, false));
        let mut fine = failed.clone();
        fine["exit_code"] = json!(0);
        assert!(!retry_needed(&fine, false));
        let mut killed = failed.clone();
        killed["reason"] = json!("killed");
        assert!(!retry_needed(&killed, false));
        assert!(!retry_needed(&json!({}), false));
    }

    #[test]
    fn claude_settings_carry_its_hooks_and_memory_off() {
        let value = claude_settings(
            Path::new("/apps/Kompitum/kompitum-app.exe"),
            Path::new("/apps/kom/kom.exe"),
            Path::new("/home/kompitum"),
        );
        assert_eq!(value["autoMemoryEnabled"], json!(false));
        let entries = value["hooks"]["SessionStart"]
            .as_array()
            .expect("SessionStart is an array");
        assert_eq!(entries.len(), 1);
        assert!(entries[0].get("matcher").is_none(), "no matcher");
        let hooks = entries[0]["hooks"].as_array().unwrap();
        assert_eq!(hooks.len(), 2);
        assert_eq!(
            hooks[0],
            json!({
                "type": "command",
                "command": "/apps/Kompitum/kompitum-app.exe",
                "args": ["hook", "SessionStart", "/home/kompitum"],
                "timeout": 10,
            })
        );
        assert_eq!(
            hooks[1],
            json!({
                "type": "command",
                "command": "/apps/kom/kom.exe",
                "args": ["vault", "memory", "--hook"],
                "timeout": 30,
            })
        );
        let command = |event: &str| {
            json!({
                "type": "command",
                "command": "/apps/Kompitum/kompitum-app.exe",
                "args": ["hook", event, "/home/kompitum"],
            })
        };
        let first = |event: &str| {
            let list = value["hooks"][event]
                .as_array()
                .unwrap_or_else(|| panic!("{event} is an array"));
            list[0].clone()
        };
        let permission = first("PermissionRequest");
        assert!(permission.get("matcher").is_none());
        let mut hook = command("PermissionRequest");
        hook["timeout"] = json!(3600);
        assert_eq!(permission["hooks"], json!([hook]));

        let question = first("PreToolUse");
        assert_eq!(question["matcher"], json!("AskUserQuestion"));
        let mut hook = command("PreToolUse");
        hook["timeout"] = json!(3600);
        assert_eq!(question["hooks"], json!([hook]));

        for event in ["UserPromptSubmit", "Stop"] {
            let entry = first(event);
            assert!(entry.get("matcher").is_none(), "{event}");
            let mut hook = command(event);
            hook["async"] = json!(true);
            assert_eq!(entry["hooks"], json!([hook]), "{event}");
        }
        let idle = value["hooks"]["Notification"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry.get("matcher") == Some(&json!("idle_prompt")))
            .expect("idle_prompt");
        let mut hook = command("Notification");
        hook["async"] = json!(true);
        assert_eq!(idle["hooks"], json!([hook]));

        assert_eq!(value["modelSettings"], model_settings());
        for model in [
            "claude-fable-5-1",
            "claude-opus-5-5",
            "claude-sonnet-5-5",
            "claude-haiku-5-5",
        ] {
            assert_eq!(
                value["modelSettings"][model],
                json!({"autoCompactWindow": AUTO_COMPACT_WINDOW})
            );
        }
        assert_eq!(
            value["modelSettings"]
                .as_object()
                .map(|m| m.len())
                .unwrap_or_default(),
            4
        );
    }

    #[test]
    fn claude_settings_install_the_chat_view_hooks() {
        let value = claude_settings(
            Path::new("/apps/Kompitum/kompitum-app.exe"),
            Path::new("/apps/kom/kom.exe"),
            Path::new("/home/kompitum"),
        );
        let command = |event: &str| {
            json!({
                "type": "command",
                "command": "/apps/Kompitum/kompitum-app.exe",
                "args": ["hook", event, "/home/kompitum"],
            })
        };
        let async_command = |event: &str| {
            let mut hook = command(event);
            hook["async"] = json!(true);
            hook
        };
        let entries = |event: &str| {
            value["hooks"][event]
                .as_array()
                .unwrap_or_else(|| panic!("{event} is an array"))
                .clone()
        };

        let display = &entries("MessageDisplay");
        assert_eq!(display.len(), 1, "{display:?}");
        assert!(display[0].get("matcher").is_none());
        let mut hook = async_command("MessageDisplay");
        hook["timeout"] = json!(10);
        assert_eq!(display[0]["hooks"], json!([hook]));

        let pre = entries("PreToolUse");
        assert_eq!(pre.len(), 2, "AskUserQuestion stays, and every tool is hooked: {pre:?}");
        assert_eq!(pre[0]["matcher"], json!("AskUserQuestion"));
        let mut ask = command("PreToolUse");
        ask["timeout"] = json!(3600);
        assert_eq!(pre[0]["hooks"], json!([ask]));
        assert!(pre[1].get("matcher").is_none(), "every tool, no matcher");
        assert_eq!(pre[1]["hooks"], json!([async_command("PreToolUse")]));

        let post = entries("PostToolUse");
        assert_eq!(post.len(), 1);
        assert!(post[0].get("matcher").is_none());
        assert_eq!(post[0]["hooks"], json!([async_command("PostToolUse")]));

        for event in ["UserPromptSubmit", "Stop", "StopFailure", "Elicitation"] {
            let list = entries(event);
            assert_eq!(list.len(), 1, "{event}");
            assert!(list[0].get("matcher").is_none(), "{event}");
            assert_eq!(list[0]["hooks"], json!([async_command(event)]), "{event}");
        }

        let notes = entries("Notification");
        let matchers: Vec<_> = notes
            .iter()
            .map(|entry| entry.get("matcher").and_then(Value::as_str).unwrap_or(""))
            .collect();
        for matcher in [
            "idle_prompt",
            "elicitation_dialog",
            "elicitation_url_dialog",
            "agent_needs_input",
            "quota_auto_resume_stale",
            "auth_success",
        ] {
            assert!(
                matchers.contains(&matcher),
                "missing Notification matcher {matcher}: {matchers:?}"
            );
        }
        for entry in &notes {
            assert_eq!(entry["hooks"], json!([async_command("Notification")]), "{entry}");
        }
    }

    #[test]
    fn setup_builds_the_two_claude_mcp_argvs() {
        let (get, add) = mcp_argvs(Path::new("/apps/kom"));
        assert_eq!(get, ["mcp", "get", "kompitum"]);
        assert_eq!(
            add,
            [
                "mcp",
                "add",
                "--scope",
                "user",
                "kompitum",
                "--",
                "/apps/kom",
                "mcp"
            ]
        );
    }

    #[test]
    fn the_skill_is_installed_only_when_absent() {
        let home = std::env::temp_dir().join(format!(
            "kompitum-lead-skill-{}-{}",
            std::process::id(),
            now_nanos()
        ));
        let path = home.join(".claude/skills/kompitum-core/SKILL.md");
        let _ = std::fs::remove_dir_all(&home);
        assert_eq!(install_skill(&home, SKILL), Ok("installed"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), SKILL);
        std::fs::write(&path, "the person's own").unwrap();
        assert_eq!(install_skill(&home, SKILL), Ok("present"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "the person's own");
        std::fs::remove_dir_all(&home).unwrap();
    }

    fn the_app_claude_settings_hold_only_the_compaction_window() {
        let dir = std::env::temp_dir().join(format!(
            "kompitum-app-settings-{}-{}",
            std::process::id(),
            now_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = write_app_claude_settings(&dir).unwrap();
        assert_eq!(path, dir.join("claude-settings.json"));
        let text = std::fs::read_to_string(&path).unwrap();
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value, json!({ "modelSettings": model_settings() }));
        assert_eq!(value.as_object().map(|m| m.len()), Some(1));
        let _ = std::fs::remove_dir_all(&dir);

        let not_a_folder = std::env::temp_dir().join(format!(
            "kompitum-app-settings-file-{}-{}",
            std::process::id(),
            now_nanos()
        ));
        std::fs::write(&not_a_folder, "not a folder").unwrap();
        let error = write_app_claude_settings(&not_a_folder).unwrap_err();
        assert!(
            error.starts_with("Couldn't write Claude Code's settings file "),
            "{error}"
        );
        assert!(
            error.contains(&not_a_folder.join("claude-settings.json").display().to_string()),
            "{error}"
        );
        let _ = std::fs::remove_file(&not_a_folder);
    }

    #[test]
    fn a_local_claude_start_gets_the_app_settings_file() {
        let file = Path::new("/apps/Kompitum/claude-settings.json");
        let gets = |argv: &[&str], on: Option<&str>| {
            with_claude_settings(
                argv.iter().map(|s| s.to_string()).collect(),
                on,
                file,
            )
        };
        assert_eq!(
            gets(&["claude"], None),
            ["claude", "--settings", "/apps/Kompitum/claude-settings.json"]
        );
        assert_eq!(
            gets(&["claude", "--resume", "abc"], None),
            [
                "claude",
                "--settings",
                "/apps/Kompitum/claude-settings.json",
                "--resume",
                "abc"
            ]
        );
        assert_eq!(
            gets(&["claude", "--settings", "/mine.json"], None),
            ["claude", "--settings", "/mine.json"],
            "its own --settings stays"
        );
        assert_eq!(
            gets(&["claude"], Some("vps")),
            ["claude"],
            "another machine keeps its own settings"
        );
        assert_eq!(
            gets(&["codex"], None),
            ["codex"],
            "another program stays as it is"
        );
        assert_eq!(
            gets(&[r"C:\Users\me\AppData\Roaming\npm\claude.exe"], None),
            [
                r"C:\Users\me\AppData\Roaming\npm\claude.exe",
                "--settings",
                "/apps/Kompitum/claude-settings.json"
            ]
        );
        assert_eq!(
            gets(&["CLAUDE.CMD"], None),
            ["CLAUDE.CMD", "--settings", "/apps/Kompitum/claude-settings.json"]
        );
        assert_eq!(gets(&[], None), Vec::<String>::new());
        assert_eq!(gets(&["notclaude"], None), ["notclaude"]);
    }

    #[test]
    fn a_session_start_from_the_lead_records_its_id_and_folder() {
        let mut streams = json!({"claude": {"folder": "/home/me", "session": "old"}});
        assert!(record_session_start(
            &mut streams,
            true,
            true,
            "SessionStart",
            Some("new-session"),
            "/home/me"
        ));
        assert_eq!(
            streams,
            json!({"claude": {"folder": "/home/me", "session": "new-session"}})
        );
        assert!(!record_session_start(
            &mut streams,
            true,
            true,
            "SessionStart",
            Some("new-session"),
            "/home/me"
        ));
        let mut fresh = json!({});
        assert!(record_session_start(
            &mut fresh,
            true,
            true,
            "SessionStart",
            Some("one"),
            "/home/me"
        ));
        assert_eq!(
            fresh,
            json!({"claude": {"folder": "/home/me", "session": "one"}}),
            "a missing record is written with its folder"
        );
        assert!(record_session_start(
            &mut fresh,
            true,
            true,
            "SessionStart",
            Some("two"),
            "/home/work"
        ));
        assert_eq!(
            fresh,
            json!({"claude": {"folder": "/home/work", "session": "two"}})
        );
        assert!(!record_session_start(
            &mut fresh,
            false,
            true,
            "SessionStart",
            Some("three"),
            "/home/me"
        ));
        assert!(!record_session_start(
            &mut fresh,
            true,
            false,
            "SessionStart",
            Some("three"),
            "/home/me"
        ));
        assert!(!record_session_start(
            &mut fresh,
            true,
            true,
            "Stop",
            Some("three"),
            "/home/me"
        ));
        assert_eq!(
            fresh,
            json!({"claude": {"folder": "/home/work", "session": "two"}})
        );
    }

    fn ask(kind: AskKind, input: Value) -> Ask {
        Ask { id: "a1".into(), terminal: "t1".into(), kind, input }
    }

    fn permission_input() -> Value {
        json!({
            "tool_name": "Bash",
            "tool_input": {"command": "cargo test"},
            "permission_suggestions": [
                {"type": "addRules", "destination": "session", "rules": [{"toolName": "Bash"}]}
            ],
        })
    }

    #[test]
    fn a_blocking_event_becomes_the_right_kind_of_ask() {
        assert_eq!(
            ask_kind("PermissionRequest", &json!({"tool_name": "ExitPlanMode", "tool_input": {}})),
            Some(AskKind::Plan)
        );
        assert_eq!(
            ask_kind("PermissionRequest", &json!({"tool_name": "Bash", "tool_input": {}})),
            Some(AskKind::Permission)
        );
        assert_eq!(
            ask_kind("PreToolUse", &json!({"tool_name": "AskUserQuestion", "tool_input": {}})),
            Some(AskKind::Question)
        );
        assert_eq!(ask_kind("PreToolUse", &json!({"tool_name": "Bash"})), Some(AskKind::Permission));
        assert_eq!(ask_kind("Stop", &json!({})), None);
        assert_eq!(ask_kind("UserPromptSubmit", &json!({})), None);
    }

    #[test]
    fn a_permission_allow_answers_the_hook() {
        let ask = ask(AskKind::Permission, permission_input());
        assert_eq!(
            hook_output(&ask, &Answer::Allow),
            Some(json!({
                "hookSpecificOutput": {
                    "hookEventName": "PermissionRequest",
                    "decision": {"behavior": "allow"},
                },
            }))
        );
    }

    #[test]
    fn allow_always_carries_the_permission_suggestions_unchanged() {
        let input = permission_input();
        let ask = ask(AskKind::Permission, input.clone());
        assert_eq!(
            hook_output(&ask, &Answer::AllowAlways),
            Some(json!({
                "hookSpecificOutput": {
                    "hookEventName": "PermissionRequest",
                    "decision": {
                        "behavior": "allow",
                        "updatedPermissions": input["permission_suggestions"],
                    },
                },
            }))
        );
    }

    #[test]
    fn a_permission_deny_carries_its_note_or_the_default() {
        let ask = ask(AskKind::Permission, permission_input());
        let deny = |note: Option<&str>| {
            Some(json!({
                "hookSpecificOutput": {
                    "hookEventName": "PermissionRequest",
                    "decision": {
                        "behavior": "deny",
                        "message": note.unwrap_or("The Owner declined."),
                    },
                },
            }))
        };
        assert_eq!(
            hook_output(&ask, &Answer::Deny { note: Some("Not that file.".into()) }),
            deny(Some("Not that file."))
        );
        assert_eq!(hook_output(&ask, &Answer::Deny { note: None }), deny(None));
        assert_eq!(
            hook_output(&ask, &Answer::Deny { note: Some("   ".into()) }),
            deny(None),
            "a blank note is no note"
        );
    }

    #[test]
    fn a_plan_approval_returns_its_tool_input_unchanged() {
        let input = json!({
            "tool_name": "ExitPlanMode",
            "tool_input": {"plan": "Do the thing.", "steps": ["one", "two"]},
        });
        let ask = ask(AskKind::Plan, input.clone());
        assert_eq!(
            hook_output(&ask, &Answer::Approve),
            Some(json!({
                "hookSpecificOutput": {
                    "hookEventName": "PermissionRequest",
                    "decision": {
                        "behavior": "allow",
                        "updatedInput": input["tool_input"],
                    },
                },
            }))
        );
    }

    #[test]
    fn keep_planning_carries_its_note_as_the_message() {
        let input = json!({"tool_name": "ExitPlanMode", "tool_input": {"plan": "x"}});
        let ask = ask(AskKind::Plan, input);
        let keep = |note: &str| {
            Some(json!({
                "hookSpecificOutput": {
                    "hookEventName": "PermissionRequest",
                    "decision": {"behavior": "deny", "message": note},
                },
            }))
        };
        assert_eq!(
            hook_output(&ask, &Answer::KeepPlanning { note: Some("Check the tests first.".into()) }),
            keep("Check the tests first.")
        );
        assert_eq!(hook_output(&ask, &Answer::KeepPlanning { note: None }), keep(""));
    }

    #[test]
    fn answers_to_a_question_are_joined_and_returned_with_it() {
        let questions = json!([
            {
                "question": "Which database?",
                "header": "Database",
                "multiSelect": false,
                "options": [{"label": "Postgres", "description": "The server's"}, {"label": "SQLite", "description": "A file"}],
            },
            {
                "question": "Which checks?",
                "header": "Checks",
                "multiSelect": true,
                "options": [{"label": "Tests", "description": "Run them"}, {"label": "Lint", "description": "Keep it tidy"}],
            },
        ]);
        let input = json!({"tool_name": "AskUserQuestion", "tool_input": {"questions": questions}});
        let ask = ask(AskKind::Question, input.clone());
        let answers = Map::from_iter([
            ("Which database?".into(), json!("Postgres")),
            ("Which checks?".into(), json!(["Tests", "Lint"])),
        ]);
        assert_eq!(
            hook_output(&ask, &Answer::Answers(answers)),
            Some(json!({
                "hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "permissionDecision": "allow",
                    "updatedInput": {
                        "questions": input["tool_input"]["questions"],
                        "answers": {
                            "Which database?": "Postgres",
                            "Which checks?": "Tests, Lint",
                        },
                    },
                },
            }))
        );
    }

    #[test]
    fn answering_in_the_terminal_prints_nothing() {
        for kind in [AskKind::Permission, AskKind::Plan, AskKind::Question] {
            assert_eq!(hook_output(&ask(kind, permission_input()), &Answer::Terminal), None);
        }
    }

    #[test]
    fn an_answer_is_read_from_what_the_composer_sends() {
        assert_eq!(Answer::from_json(&json!({"action": "allow"})), Some(Answer::Allow));
        assert_eq!(Answer::from_json(&json!({"action": "allow_always"})), Some(Answer::AllowAlways));
        assert_eq!(
            Answer::from_json(&json!({"action": "deny", "note": "no"})),
            Some(Answer::Deny { note: Some("no".into()) })
        );
        assert_eq!(Answer::from_json(&json!({"action": "approve"})), Some(Answer::Approve));
        assert_eq!(
            Answer::from_json(&json!({"action": "keep_planning", "note": "wait"})),
            Some(Answer::KeepPlanning { note: Some("wait".into()) })
        );
        assert_eq!(
            Answer::from_json(&json!({"action": "answers", "answers": {"Q?": "A"}})),
            Some(Answer::Answers(Map::from_iter([("Q?".into(), json!("A"))])))
        );
        assert_eq!(Answer::from_json(&json!({"action": "terminal"})), Some(Answer::Terminal));
        assert_eq!(Answer::from_json(&json!({"action": "what"})), None);
        assert_eq!(Answer::from_json(&json!({})), None);
    }
}
