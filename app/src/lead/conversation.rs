//! The conversation the panel draws. `fold` applies one hook event; it does no IO.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// Commands that open one of Claude's own menus when sent with no argument.
pub const PICKER_COMMANDS: &[&str] = &[
    "/model",
    "/resume",
    "/rewind",
    "/config",
    "/login",
    "/permissions",
    "/agents",
    "/mcp",
    "/memory",
    "/hooks",
    "/theme",
    "/status",
];

/// Items the panel draws, plus whether the terminal is shown and why.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Conversation {
    #[serde(default)]
    pub items: Vec<Item>,
    #[serde(default)]
    pub version: u64,
    #[serde(default)]
    next_id: u64,
    #[serde(default)]
    pub terminal: TerminalVis,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reveal: Option<Reveal>,
    /// The one-line note an automatic reveal shows above the composer box.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reveal_why: Option<String>,
    /// Assistant messages by `message_id`. Not saved: a resumed lead reads its transcript.
    #[serde(skip)]
    messages: BTreeMap<String, Message>,
}

/// One assistant message while its batches arrive. Batches can come out of order,
/// since each hook runs on its own, so they are kept by index until they join up.
#[derive(Clone, Debug, Default, PartialEq)]
struct Message {
    /// The id of the `Item::Assistant` it draws into.
    item: String,
    batches: BTreeMap<u64, String>,
    /// The index of the batch marked final, once it has arrived.
    last: Option<u64>,
    /// Set by `Stop`, which ends every message even when its last batch is late.
    stopped: bool,
}

/// What `lead()` publishes about the conversation, without the items.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationMeta {
    pub version: u64,
    pub length: usize,
}

/// Whether the lead's terminal pane is drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TerminalVis {
    #[default]
    Hidden,
    Shown,
}

/// Why the terminal is shown, and how it hides.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Reveal {
    Login,
    Auto,
    Manual,
    Startup,
}

/// One row in the conversation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Item {
    Owner {
        id: String,
        text: String,
    },
    Assistant {
        id: String,
        text: String,
        done: bool,
    },
    Tool {
        id: String,
        name: String,
        summary: String,
        state: ToolState,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        result: Option<String>,
    },
    Notice {
        id: String,
        text: String,
    },
}

/// A tool's progress as the panel draws it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolState {
    Running,
    Done,
    Failed,
}

/// A hook event, or a local one (`History`, `Toggle`), for `fold`.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    UserPromptSubmit {
        prompt: String,
    },
    MessageDisplay {
        message_id: String,
        index: u64,
        last: bool,
        delta: String,
    },
    PreToolUse {
        id: String,
        name: String,
        input: Value,
    },
    PostToolUse {
        id: String,
        name: String,
        input: Value,
        result: String,
    },
    PostToolUseFailure {
        id: String,
        name: String,
        input: Value,
        result: String,
    },
    Stop,
    StopFailure {
        error: Option<String>,
    },
    Notification {
        kind: String,
    },
    Elicitation,
    History {
        items: Vec<Item>,
    },
    HistoryUnread,
    Toggle {
        show: Option<bool>,
    },
}

/// What reading a transcript produced.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TranscriptLoad {
    pub items: Vec<Item>,
    pub skipped: u32,
}

impl Conversation {
    pub fn meta(&self) -> ConversationMeta {
        ConversationMeta {
            version: self.version,
            length: self.items.len(),
        }
    }

    pub fn items_from(&self, from: usize) -> &[Item] {
        let from = from.min(self.items.len());
        &self.items[from..]
    }

    pub(crate) fn next(&mut self, kind: &str) -> String {
        self.next_id += 1;
        format!("{kind}-{}", self.next_id)
    }

    pub(crate) fn touched(&mut self) {
        self.version += 1;
    }

    pub(crate) fn hide(&mut self) {
        if self.terminal != TerminalVis::Hidden || self.reveal.is_some() || self.reveal_why.is_some()
        {
            self.terminal = TerminalVis::Hidden;
            self.reveal = None;
            self.reveal_why = None;
            self.touched();
        }
    }

    pub(crate) fn show(&mut self, reveal: Reveal, why: Option<String>) {
        let why_same = self.reveal_why == why;
        if self.terminal == TerminalVis::Shown && self.reveal == Some(reveal) && why_same {
            return;
        }
        self.terminal = TerminalVis::Shown;
        self.reveal = Some(reveal);
        self.reveal_why = why;
        self.touched();
    }
}

/// Applies one event. No IO.
pub fn fold(mut conversation: Conversation, event: &Event) -> Conversation {
    match event {
        Event::UserPromptSubmit { prompt } => on_submit(&mut conversation, prompt),
        Event::MessageDisplay {
            message_id,
            index,
            last,
            delta,
        } => on_display(&mut conversation, message_id, *index, *last, delta),
        Event::PreToolUse { id, name, input } => on_pre_tool(&mut conversation, id, name, input),
        Event::PostToolUse {
            id,
            name,
            input,
            result,
        } => on_tool_end(&mut conversation, id, name, input, result, ToolState::Done),
        Event::PostToolUseFailure {
            id,
            name,
            input,
            result,
        } => on_tool_end(&mut conversation, id, name, input, result, ToolState::Failed),
        Event::Stop => on_stop(&mut conversation),
        Event::StopFailure { error } => on_stop_failure(&mut conversation, error.as_deref()),
        Event::Notification { kind } => on_notification(&mut conversation, kind),
        Event::Elicitation => {
            if conversation.reveal != Some(Reveal::Manual) {
                conversation.show(Reveal::Auto, Some("Claude needs your input.".into()));
            }
        }
        Event::History { items } => {
            conversation.items = items.clone();
            conversation.messages.clear();
            conversation.next_id = conversation.next_id.max(items.len() as u64);
            conversation.touched();
        }
        Event::HistoryUnread => {
            let id = conversation.next("notice");
            conversation.items.push(Item::Notice {
                id,
                text: "Couldn't read the conversation history.".into(),
            });
            conversation.touched();
        }
        Event::Toggle { show } => on_toggle(&mut conversation, *show),
    }
    conversation
}

/// The hook name and its JSON, or None when the event is not for the conversation
/// (a permission, plan or question card, or an event we do not draw).
pub fn event_from_hook(event: &str, input: &Value) -> Option<Event> {
    match event {
        "UserPromptSubmit" => Some(Event::UserPromptSubmit {
            prompt: string_field(input, "prompt"),
        }),
        "MessageDisplay" => Some(Event::MessageDisplay {
            message_id: string_field(input, "message_id"),
            index: input.get("index").and_then(Value::as_u64).unwrap_or(0),
            last: input.get("final").and_then(Value::as_bool).unwrap_or(false),
            delta: string_field(input, "delta"),
        }),
        "PreToolUse" => {
            let name = string_field(input, "tool_name");
            if name == "AskUserQuestion" {
                return None;
            }
            Some(Event::PreToolUse {
                id: tool_id(input),
                name,
                input: input.get("tool_input").cloned().unwrap_or(Value::Null),
            })
        }
        "PostToolUse" => {
            let name = string_field(input, "tool_name");
            if name == "AskUserQuestion" {
                return None;
            }
            Some(Event::PostToolUse {
                id: tool_id(input),
                name,
                input: tool_input(input),
                result: tool_result(input),
            })
        }
        "PostToolUseFailure" => {
            let name = string_field(input, "tool_name");
            if name == "AskUserQuestion" {
                return None;
            }
            // An interrupt has no error worth showing; the first line of an error is the same text as its result.
            let result = if input.get("is_interrupt").and_then(Value::as_bool).unwrap_or(false) {
                "Stopped.".to_owned()
            } else {
                string_field(input, "error")
            };
            Some(Event::PostToolUseFailure {
                id: tool_id(input),
                name,
                input: tool_input(input),
                result,
            })
        }
        "Stop" => Some(Event::Stop),
        "StopFailure" => {
            let error = string_field(input, "error");
            Some(Event::StopFailure {
                error: (!error.is_empty()).then_some(error),
            })
        }
        "Notification" => Some(Event::Notification {
            kind: string_field(input, "notification_type"),
        }),
        "Elicitation" => Some(Event::Elicitation),
        _ => None,
    }
}

/// How a new lead starts after `claude auth status` exits. Only the code is used.
pub fn reveal_after_auth_status(code: i32) -> (TerminalVis, Option<Reveal>) {
    if code == 0 {
        (TerminalVis::Hidden, None)
    } else {
        (TerminalVis::Shown, Some(Reveal::Login))
    }
}

/// A bare picker command: one of the listed `/` commands, with no argument.
pub fn is_picker_command(text: &str) -> bool {
    let text = text.trim();
    PICKER_COMMANDS.contains(&text)
}

/// Reads a Claude Code JSONL transcript into the same items `fold` builds.
/// Unknown line types are skipped and counted.
pub fn items_from_transcript(text: &str) -> TranscriptLoad {
    let mut items = Vec::new();
    let mut skipped = 0;
    let mut next_id = 0u64;
    let mut next = |kind: &str| {
        next_id += 1;
        format!("{kind}-{next_id}")
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            skipped += 1;
            continue;
        };
        match line_type(&value) {
            Some("user") => apply_user(&mut items, &mut next, &value),
            Some("assistant") => apply_assistant(&mut items, &mut next, &value),
            Some("system") if value.get("subtype").and_then(Value::as_str) == Some("compact_boundary") => {
                let id = value.get("uuid").and_then(Value::as_str).map_or_else(|| next("notice"), str::to_owned);
                items.push(Item::Notice {
                    id,
                    text: "Tidied up the conversation".into(),
                });
            }
            Some(_) => skipped += 1,
            None => skipped += 1,
        }
    }
    TranscriptLoad { items, skipped }
}

/// `/compact` sent on its own, or with a note after it: Claude's own command, not the Owner's words.
pub fn is_compact_command(text: &str) -> bool {
    text == "/compact" || text.starts_with("/compact ")
}

fn on_submit(conversation: &mut Conversation, prompt: &str) {
    if is_compact_command(prompt) {
        return;
    }
    if conversation.reveal == Some(Reveal::Auto) && !is_picker_command(prompt) {
        conversation.hide();
    }
    if !prompt.is_empty() {
        let id = conversation.next("owner");
        conversation.items.push(Item::Owner {
            id,
            text: prompt.to_owned(),
        });
        conversation.touched();
    }
    if is_picker_command(prompt) && conversation.reveal != Some(Reveal::Manual) {
        let why = format!("Claude is showing its {} menu.", prompt.trim());
        conversation.show(Reveal::Auto, Some(why));
    }
}

/// One batch of an assistant message. The item's text is the batches joined in index
/// order up to the first gap, so a late batch never lands ahead of an earlier one.
fn on_display(conversation: &mut Conversation, message_id: &str, index: u64, last: bool, delta: &str) {
    if !conversation.messages.contains_key(message_id) {
        let id = conversation.next("assistant");
        conversation.items.push(Item::Assistant {
            id: id.clone(),
            text: String::new(),
            done: false,
        });
        conversation.messages.insert(
            message_id.to_owned(),
            Message {
                item: id,
                ..Message::default()
            },
        );
    }
    let Some(message) = conversation.messages.get_mut(message_id) else {
        return;
    };
    message.batches.insert(index, delta.to_owned());
    if last {
        message.last = Some(index);
    }
    let (text, done) = assembled(message);
    let item_id = message.item.clone();
    if let Some(Item::Assistant {
        text: body,
        done: have_done,
        ..
    }) = conversation.items.iter_mut().find(|item| matches!(item, Item::Assistant { id, .. } if *id == item_id))
    {
        *body = text;
        *have_done = done;
    }
    conversation.touched();
}

/// The text of the batches from index 0 up to the first missing one, and whether the
/// final batch and every batch below it have arrived.
fn assembled(message: &Message) -> (String, bool) {
    let mut text = String::new();
    let mut index = 0;
    while let Some(part) = message.batches.get(&index) {
        text.push_str(part);
        index += 1;
    }
    let complete = message.last.is_some_and(|last| index > last);
    (text, complete || message.stopped)
}

fn on_pre_tool(conversation: &mut Conversation, id: &str, name: &str, input: &Value) {
    if let Some(Item::Tool {
        name: have,
        summary: have_summary,
        state,
        ..
    }) = tool_mut(conversation, id)
    {
        // A Pre that arrives after its Post or Failure must not start the tool again.
        if *state == ToolState::Running {
            *have = name.to_owned();
            *have_summary = tool_summary(name, input, ToolState::Running);
            conversation.touched();
        }
        return;
    }
    let id = if id.is_empty() {
        conversation.next("tool")
    } else {
        id.to_owned()
    };
    conversation.items.push(Item::Tool {
        id,
        name: name.to_owned(),
        summary: tool_summary(name, input, ToolState::Running),
        state: ToolState::Running,
        result: None,
    });
    conversation.touched();
}

/// The summary of a tool that has ended: built again from the end's own input when it has one, otherwise the
/// running words turned into the past ("Running `ls`" becomes "Ran `ls`").
fn ended_summary(current: &str, name: &str, input: &Value, state: ToolState) -> String {
    if input.as_object().is_some_and(|fields| !fields.is_empty()) {
        return tool_summary(name, input, state);
    }
    match current.strip_prefix("Running ") {
        Some(rest) => format!("Ran {rest}"),
        None => current.to_owned(),
    }
}

/// A tool's end, from its Post or its Failure. The Pre may not have arrived yet, so
/// the item is made here when it is missing.
fn on_tool_end(
    conversation: &mut Conversation,
    id: &str,
    name: &str,
    input: &Value,
    result: &str,
    state: ToolState,
) {
    let clipped = clip_result(result);
    let result = if clipped.is_empty() { None } else { Some(clipped) };
    if let Some(Item::Tool {
        name: have_name,
        summary: have_summary,
        state: have,
        result: have_result,
        ..
    }) = tool_mut(conversation, id)
    {
        *have_summary = ended_summary(have_summary, have_name, input, state);
        *have = state;
        *have_result = result;
        conversation.touched();
        return;
    }
    // Without an id, the running tool of this name is the one that ended.
    if id.is_empty() {
        if let Some(Item::Tool {
            summary: have_summary,
            state: have,
            result: have_result,
            ..
        }) = conversation.items.iter_mut().rev().find(|item| {
            matches!(
                item,
                Item::Tool {
                    name: have_name,
                    state: ToolState::Running,
                    ..
                } if have_name == name
            )
        }) {
            *have_summary = ended_summary(have_summary, name, input, state);
            *have = state;
            *have_result = result;
            conversation.touched();
            return;
        }
    }
    let id = if id.is_empty() {
        conversation.next("tool")
    } else {
        id.to_owned()
    };
    conversation.items.push(Item::Tool {
        id,
        name: name.to_owned(),
        summary: tool_summary(name, input, state),
        state,
        result,
    });
    conversation.touched();
}

fn on_stop(conversation: &mut Conversation) {
    if finish_assistant(conversation) {
        conversation.touched();
    }
    if conversation.reveal == Some(Reveal::Auto) {
        conversation.hide();
    }
}

fn on_stop_failure(conversation: &mut Conversation, error: Option<&str>) {
    let _ = finish_assistant(conversation);
    let id = conversation.next("notice");
    conversation.items.push(Item::Notice {
        id,
        text: stop_failure_text(error),
    });
    conversation.touched();
    if error == Some("authentication_failed") {
        conversation.show(Reveal::Login, Some("Claude needs you to sign in.".into()));
    }
}

/// The notice for a turn that stopped on an error, in the app's own words.
fn stop_failure_text(error: Option<&str>) -> String {
    let words = match error {
        Some("rate_limit") => "rate limit".into(),
        Some("overloaded") => "Claude is overloaded".into(),
        Some("authentication_failed") => "sign-in failed".into(),
        Some("billing_error") => "a billing problem".into(),
        Some("max_output_tokens") => "the reply hit its length limit".into(),
        Some(other) if !other.is_empty() => other.replace('_', " "),
        _ => return "The turn stopped.".into(),
    };
    format!("The turn stopped: {words}.")
}

fn on_notification(conversation: &mut Conversation, kind: &str) {
    match kind {
        "auth_success" => {
            let id = conversation.next("notice");
            conversation.items.push(Item::Notice {
                id,
                text: "Signed in.".into(),
            });
            conversation.touched();
            if conversation.reveal == Some(Reveal::Login) {
                conversation.hide();
            }
        }
        // Sign-in worked but could not be saved: the login reveal stays, so the Owner can act on it.
        "auth_storage_failure" => {
            let id = conversation.next("notice");
            conversation.items.push(Item::Notice {
                id,
                text: "Signed in, but the sign-in couldn't be saved.".into(),
            });
            conversation.touched();
        }
        "elicitation_dialog" | "elicitation_url_dialog" => {
            if conversation.reveal != Some(Reveal::Manual) {
                conversation.show(Reveal::Auto, Some("Claude needs your input.".into()));
            }
        }
        "agent_needs_input" => {
            if conversation.reveal != Some(Reveal::Manual) {
                conversation.show(Reveal::Auto, Some("Claude needs your input.".into()));
            }
        }
        "quota_auto_resume_stale" => {
            if conversation.reveal != Some(Reveal::Manual) {
                conversation.show(
                    Reveal::Auto,
                    Some("Claude is waiting for you to continue.".into()),
                );
            }
        }
        _ => {}
    }
}

fn on_toggle(conversation: &mut Conversation, show: Option<bool>) {
    let show = show.unwrap_or(conversation.terminal != TerminalVis::Shown);
    if show {
        conversation.show(Reveal::Manual, None);
    } else {
        conversation.hide();
    }
}

fn finish_assistant(conversation: &mut Conversation) -> bool {
    let mut changed = false;
    for message in conversation.messages.values_mut() {
        message.stopped = true;
    }
    for item in &mut conversation.items {
        if let Item::Assistant { done, .. } = item {
            if !*done {
                *done = true;
                changed = true;
            }
        }
    }
    changed
}

fn tool_mut<'a>(conversation: &'a mut Conversation, id: &str) -> Option<&'a mut Item> {
    if id.is_empty() {
        return None;
    }
    conversation.items.iter_mut().find(|item| match item {
        Item::Tool { id: have, .. } => have == id,
        _ => false,
    })
}

/// A one-line summary from a tool's name and input.
pub fn tool_summary(name: &str, input: &Value, state: ToolState) -> String {
    let path = |keys: &[&str]| {
        keys.iter()
            .find_map(|key| input.get(*key).and_then(Value::as_str))
            .unwrap_or("")
    };
    match name {
        "Read" => {
            let file = path(&["file_path", "path"]);
            if file.is_empty() {
                "Read a file".into()
            } else {
                format!("Read {file}")
            }
        }
        "Write" | "Edit" | "NotebookEdit" | "MultiEdit" => edited_summary(input),
        "Bash" => {
            let command = path(&["command"]);
            let running = state == ToolState::Running;
            match (command.is_empty(), running) {
                (true, true) => "Running a command".into(),
                (true, false) => "Ran a command".into(),
                (false, true) => format!("Running `{command}`"),
                (false, false) => format!("Ran `{command}`"),
            }
        }
        "Grep" => {
            let pattern = path(&["pattern"]);
            if pattern.is_empty() {
                "Grep".into()
            } else {
                format!("Grep {pattern}")
            }
        }
        "Glob" => {
            let pattern = path(&["pattern"]);
            if pattern.is_empty() {
                "Glob".into()
            } else {
                format!("Glob {pattern}")
            }
        }
        "WebFetch" => {
            let url = path(&["url"]);
            if url.is_empty() {
                "Fetch a url".into()
            } else {
                format!("Fetch {url}")
            }
        }
        "WebSearch" => {
            let query = path(&["query"]);
            if query.is_empty() {
                "Search the web".into()
            } else {
                format!("Search the web for {query}")
            }
        }
        other => other.to_owned(),
    }
}

fn edited_summary(input: &Value) -> String {
    if let Some(files) = input.get("files").and_then(Value::as_array) {
        let paths: Vec<&str> = files.iter().filter_map(Value::as_str).collect();
        return match paths.as_slice() {
            [] => "Edited files".into(),
            [one] => format!("Edited {one}"),
            many => format!("Edited {} files", many.len()),
        };
    }
    match input
        .get("file_path")
        .or_else(|| input.get("path"))
        .and_then(Value::as_str)
    {
        Some(path) if !path.is_empty() => format!("Edited {path}"),
        _ => "Edited a file".into(),
    }
}

fn clip_result(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= 12 {
        return text.to_owned();
    }
    lines[..12].join("\n")
}

fn string_field(input: &Value, key: &str) -> String {
    input
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned()
}

fn first_string(input: &Value, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|key| input.get(*key).and_then(Value::as_str))
        .unwrap_or("")
        .to_owned()
}

fn tool_id(input: &Value) -> String {
    first_string(input, &["tool_use_id", "id"])
}

fn tool_input(input: &Value) -> Value {
    input.get("tool_input").cloned().unwrap_or(Value::Null)
}

/// A tool's result text from its `tool_response`, which each tool shapes itself. A
/// Bash response carries `stdout` and `stderr`; an MCP one carries `content`. Anything
/// else has no text to show, and is not dumped as JSON.
fn tool_result(input: &Value) -> String {
    match input.get("tool_response") {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Object(map)) => {
            let streams: Vec<&str> = ["stdout", "stderr"]
                .iter()
                .filter_map(|key| map.get(*key).and_then(Value::as_str))
                .map(str::trim_end)
                .filter(|text| !text.is_empty())
                .collect();
            if streams.is_empty() {
                content_text(map.get("content"))
            } else {
                streams.join("\n")
            }
        }
        _ => String::new(),
    }
}

/// Text from a `content` value: a string, or an array whose text parts join by lines.
fn content_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts.iter().filter_map(part_text).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    }
}

fn line_type(value: &Value) -> Option<&str> {
    value
        .get("type")
        .and_then(Value::as_str)
        .or_else(|| value.pointer("/message/role").and_then(Value::as_str))
}

fn message_content(value: &Value) -> &Value {
    value
        .pointer("/message/content")
        .or_else(|| value.get("content"))
        .unwrap_or(&Value::Null)
}

fn apply_user(items: &mut Vec<Item>, next: &mut impl FnMut(&str) -> String, value: &Value) {
    // A compaction's summary and Claude's meta lines are written by Claude, not the Owner.
    if value["isCompactSummary"] == true || value["isMeta"] == true {
        return;
    }
    match message_content(value) {
        Value::String(text) if !text.is_empty() => {
            items.push(Item::Owner {
                id: next("owner"),
                text: text.clone(),
            });
        }
        Value::Array(parts) => {
            let mut texts = Vec::new();
            for part in parts {
                let kind = part.get("type").and_then(Value::as_str).unwrap_or("");
                if kind == "tool_result" {
                    flush_owner(items, next, &mut texts);
                    apply_tool_result(items, part);
                } else if let Some(text) = part_text(part) {
                    texts.push(text);
                }
            }
            flush_owner(items, next, &mut texts);
        }
        _ => {}
    }
}

fn flush_owner(items: &mut Vec<Item>, next: &mut impl FnMut(&str) -> String, texts: &mut Vec<String>) {
    if texts.is_empty() {
        return;
    }
    items.push(Item::Owner {
        id: next("owner"),
        text: texts.join(""),
    });
    texts.clear();
}

fn apply_assistant(items: &mut Vec<Item>, next: &mut impl FnMut(&str) -> String, value: &Value) {
    match message_content(value) {
        Value::String(text) if !text.is_empty() => append_assistant(items, next, text),
        Value::Array(parts) => {
            for part in parts {
                let kind = part.get("type").and_then(Value::as_str).unwrap_or("");
                if kind == "tool_use" {
                    let id = part
                        .get("id")
                        .and_then(Value::as_str)
                        .filter(|id| !id.is_empty())
                        .map(str::to_owned)
                        .unwrap_or_else(|| next("tool"));
                    let name = part.get("name").and_then(Value::as_str).unwrap_or("tool");
                    let input = part.get("input").cloned().unwrap_or(Value::Null);
                    items.push(Item::Tool {
                        id,
                        name: name.to_owned(),
                        summary: tool_summary(name, &input, ToolState::Running),
                        state: ToolState::Running,
                        result: None,
                    });
                } else if kind == "thinking" {
                    continue;
                } else if let Some(text) = part_text(part) {
                    append_assistant(items, next, &text);
                }
            }
        }
        _ => {}
    }
}

fn append_assistant(items: &mut Vec<Item>, next: &mut impl FnMut(&str) -> String, text: &str) {
    if text.is_empty() {
        return;
    }
    if let Some(Item::Assistant { text: body, done, .. }) = items.last_mut() {
        if !*done {
            body.push_str(text);
            return;
        }
    }
    items.push(Item::Assistant {
        id: next("assistant"),
        text: text.to_owned(),
        done: true,
    });
}

fn apply_tool_result(items: &mut Vec<Item>, part: &Value) {
    let id = part.get("tool_use_id").and_then(Value::as_str).unwrap_or("");
    let result = match part.get("content") {
        Some(content) => content_text(Some(content)),
        None => part.get("text").and_then(Value::as_str).unwrap_or("").to_owned(),
    };
    let failed = part.get("is_error").and_then(Value::as_bool).unwrap_or(false);
    let state = if failed { ToolState::Failed } else { ToolState::Done };
    let clipped = clip_result(&result);
    if let Some(Item::Tool {
        name: have_name,
        summary: have_summary,
        state: have,
        result: have_result,
        ..
    }) = items.iter_mut().find(|item| matches!(item, Item::Tool { id: have, .. } if have == id))
    {
        *have_summary = ended_summary(have_summary, have_name, &Value::Null, state);
        *have = state;
        *have_result = if clipped.is_empty() { None } else { Some(clipped) };
    }
}

fn part_text(part: &Value) -> Option<String> {
    if part.get("type").and_then(Value::as_str) == Some("text")
        || part.get("type").is_none()
    {
        part.get("text").and_then(Value::as_str).map(str::to_owned)
    } else {
        None
    }
}

#[cfg(test)]
#[path = "conversation/tests.rs"]
mod tests;
