//! The conversation the panel draws. `fold` applies one hook event; it does no IO.

use serde::{Deserialize, Serialize};
use serde_json::Value;

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
        text: String,
    },
    PreToolUse {
        id: String,
        name: String,
        input: Value,
    },
    PostToolUse {
        id: String,
        name: String,
        result: String,
        failed: bool,
    },
    Stop,
    StopFailure {
        matcher: Option<String>,
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

    fn next(&mut self, kind: &str) -> String {
        self.next_id += 1;
        format!("{kind}-{}", self.next_id)
    }

    fn touched(&mut self) {
        self.version += 1;
    }

    fn hide(&mut self) {
        if self.terminal != TerminalVis::Hidden || self.reveal.is_some() || self.reveal_why.is_some()
        {
            self.terminal = TerminalVis::Hidden;
            self.reveal = None;
            self.reveal_why = None;
            self.touched();
        }
    }

    fn show(&mut self, reveal: Reveal, why: Option<String>) {
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
        Event::MessageDisplay { text } => on_display(&mut conversation, text),
        Event::PreToolUse { id, name, input } => on_pre_tool(&mut conversation, id, name, input),
        Event::PostToolUse {
            id,
            name,
            result,
            failed,
        } => on_post_tool(&mut conversation, id, name, result, *failed),
        Event::Stop => on_stop(&mut conversation),
        Event::StopFailure { matcher } => on_stop_failure(&mut conversation, matcher.as_deref()),
        Event::Notification { kind } => on_notification(&mut conversation, kind),
        Event::Elicitation => {
            if conversation.reveal != Some(Reveal::Manual) {
                conversation.show(Reveal::Auto, Some("Claude needs your input.".into()));
            }
        }
        Event::History { items } => {
            conversation.items = items.clone();
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
        "MessageDisplay" => {
            let text = first_string(input, &["delta", "text"]);
            let text = if text.is_empty() {
                input.get("message").and_then(Value::as_str).unwrap_or("").to_owned()
            } else {
                text
            };
            Some(Event::MessageDisplay { text })
        }
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
                result: tool_result(input),
                failed: tool_failed(input),
            })
        }
        "Stop" => Some(Event::Stop),
        "StopFailure" => Some(Event::StopFailure {
            matcher: stop_failure_matcher(input),
        }),
        "Notification" => Some(Event::Notification {
            kind: first_string(input, &["notification_type", "matcher", "type"]),
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
            Some(_) => skipped += 1,
            None => skipped += 1,
        }
    }
    TranscriptLoad { items, skipped }
}

fn on_submit(conversation: &mut Conversation, prompt: &str) {
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

fn on_display(conversation: &mut Conversation, text: &str) {
    if text.is_empty() {
        return;
    }
    match conversation.items.last_mut() {
        Some(Item::Assistant { text: body, done, .. }) if !*done => {
            body.push_str(text);
            conversation.touched();
        }
        _ => {
            let id = conversation.next("assistant");
            conversation.items.push(Item::Assistant {
                id,
                text: text.to_owned(),
                done: false,
            });
            conversation.touched();
        }
    }
}

fn on_pre_tool(conversation: &mut Conversation, id: &str, name: &str, input: &Value) {
    let summary = tool_summary(name, input);
    if let Some(Item::Tool {
        name: have,
        summary: have_summary,
        state,
        ..
    }) = tool_mut(conversation, id)
    {
        *have = name.to_owned();
        *have_summary = summary;
        *state = ToolState::Running;
        conversation.touched();
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
        summary,
        state: ToolState::Running,
        result: None,
    });
    conversation.touched();
}

fn on_post_tool(conversation: &mut Conversation, id: &str, name: &str, result: &str, failed: bool) {
    let state = if failed { ToolState::Failed } else { ToolState::Done };
    let clipped = clip_result(result);
    if let Some(Item::Tool {
        state: have,
        result: have_result,
        ..
    }) = tool_mut(conversation, id)
    {
        *have = state;
        *have_result = if clipped.is_empty() {
            None
        } else {
            Some(clipped)
        };
        conversation.touched();
        return;
    }
    if let Some(Item::Tool {
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
        *have = state;
        *have_result = if clipped.is_empty() {
            None
        } else {
            Some(clipped)
        };
        conversation.touched();
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
        summary: tool_summary(name, &Value::Null),
        state,
        result: if clipped.is_empty() {
            None
        } else {
            Some(clipped)
        },
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

fn on_stop_failure(conversation: &mut Conversation, matcher: Option<&str>) {
    let _ = finish_assistant(conversation);
    let text = match matcher {
        Some(matcher) if !matcher.is_empty() => {
            format!("The turn stopped: {}.", matcher.replace('_', " "))
        }
        _ => "The turn stopped.".into(),
    };
    let id = conversation.next("notice");
    conversation.items.push(Item::Notice { id, text });
    conversation.touched();
    if matcher == Some("authentication_failed") {
        conversation.show(Reveal::Login, Some("Claude needs you to sign in.".into()));
    }
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
pub fn tool_summary(name: &str, input: &Value) -> String {
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
            if command.is_empty() {
                "Ran a command".into()
            } else {
                format!("Ran `{command}`")
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

fn tool_result(input: &Value) -> String {
    if let Some(text) = input.get("tool_response").and_then(value_text) {
        return text;
    }
    if let Some(text) = input.get("tool_output").and_then(value_text) {
        return text;
    }
    String::new()
}

fn value_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Object(map) => map
            .get("content")
            .and_then(value_text)
            .or_else(|| map.get("text").and_then(Value::as_str).map(str::to_owned)),
        Value::Array(parts) => {
            let text = parts.iter().filter_map(value_text).collect::<Vec<_>>().join("");
            if text.is_empty() { None } else { Some(text) }
        }
        _ => None,
    }
}

fn tool_failed(input: &Value) -> bool {
    input.get("is_error").and_then(Value::as_bool).unwrap_or(false)
        || input
            .get("tool_response")
            .and_then(|value| value.get("is_error"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
}

fn stop_failure_matcher(input: &Value) -> Option<String> {
    for key in ["matcher", "reason", "error_type", "subtype", "error"] {
        if let Some(text) = input.get(key).and_then(Value::as_str).filter(|text| !text.is_empty()) {
            return Some(text.to_owned());
        }
    }
    None
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
                        summary: tool_summary(name, &input),
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
    let result = part
        .get("content")
        .and_then(value_text)
        .or_else(|| part.get("text").and_then(Value::as_str).map(str::to_owned))
        .unwrap_or_default();
    let failed = part.get("is_error").and_then(Value::as_bool).unwrap_or(false);
    let state = if failed { ToolState::Failed } else { ToolState::Done };
    let clipped = clip_result(&result);
    if let Some(Item::Tool {
        state: have,
        result: have_result,
        ..
    }) = items.iter_mut().find(|item| matches!(item, Item::Tool { id: have, .. } if have == id))
    {
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
mod tests {
    use super::*;
    use serde_json::json;

    fn owner_text(item: &Item) -> &str {
        match item {
            Item::Owner { text, .. } => text,
            _ => panic!("expected owner, got {item:?}"),
        }
    }

    fn assistant(item: &Item) -> (&str, bool) {
        match item {
            Item::Assistant { text, done, .. } => (text, *done),
            _ => panic!("expected assistant, got {item:?}"),
        }
    }

    fn tool(item: &Item) -> (&str, &str, ToolState, Option<&str>) {
        match item {
            Item::Tool {
                name,
                summary,
                state,
                result,
                ..
            } => (name, summary, *state, result.as_deref()),
            _ => panic!("expected tool, got {item:?}"),
        }
    }

    fn notice(item: &Item) -> &str {
        match item {
            Item::Notice { text, .. } => text,
            _ => panic!("expected notice, got {item:?}"),
        }
    }

    #[test]
    fn a_turn_with_three_text_batches_and_two_tools() {
        let mut c = Conversation::default();
        c = fold(
            c,
            &Event::UserPromptSubmit {
                prompt: "Look at lead.rs and test it".into(),
            },
        );
        c = fold(
            c,
            &Event::MessageDisplay {
                text: "I'll read".into(),
            },
        );
        c = fold(
            c,
            &Event::MessageDisplay {
                text: " the file.\n".into(),
            },
        );
        c = fold(
            c,
            &Event::PreToolUse {
                id: "u1".into(),
                name: "Read".into(),
                input: json!({"file_path": "app/src/lead.rs"}),
            },
        );
        c = fold(
            c,
            &Event::PostToolUse {
                id: "u1".into(),
                name: "Read".into(),
                result: "pub fn fold".into(),
                failed: false,
            },
        );
        c = fold(
            c,
            &Event::PreToolUse {
                id: "u2".into(),
                name: "Bash".into(),
                input: json!({"command": "cargo test"}),
            },
        );
        c = fold(
            c,
            &Event::PostToolUse {
                id: "u2".into(),
                name: "Bash".into(),
                result: "ok\n".repeat(20),
                failed: false,
            },
        );
        c = fold(
            c,
            &Event::MessageDisplay {
                text: "Done.".into(),
            },
        );
        c = fold(c, &Event::Stop);

        assert_eq!(c.items.len(), 5, "{:?}", c.items);
        assert_eq!(owner_text(&c.items[0]), "Look at lead.rs and test it");
        assert_eq!(assistant(&c.items[1]), ("I'll read the file.\n", true));
        assert_eq!(
            tool(&c.items[2]),
            ("Read", "Read app/src/lead.rs", ToolState::Done, Some("pub fn fold"))
        );
        let (name, summary, state, result) = tool(&c.items[3]);
        assert_eq!((name, summary, state), ("Bash", "Ran `cargo test`", ToolState::Done));
        assert_eq!(result.unwrap().lines().count(), 12);
        assert_eq!(assistant(&c.items[4]), ("Done.", true));
    }

    #[test]
    fn a_failed_tool_is_marked_failed() {
        let mut c = Conversation::default();
        c = fold(
            c,
            &Event::PreToolUse {
                id: "u1".into(),
                name: "Bash".into(),
                input: json!({"command": "false"}),
            },
        );
        c = fold(
            c,
            &Event::PostToolUse {
                id: "u1".into(),
                name: "Bash".into(),
                result: "exit 1".into(),
                failed: true,
            },
        );
        assert_eq!(
            tool(&c.items[0]),
            ("Bash", "Ran `false`", ToolState::Failed, Some("exit 1"))
        );
    }

    #[test]
    fn stop_failure_adds_a_notice_from_its_matcher() {
        let c = fold(
            Conversation::default(),
            &Event::StopFailure {
                matcher: Some("rate_limit".into()),
            },
        );
        assert_eq!(notice(&c.items[0]), "The turn stopped: rate limit.");
        assert_eq!(c.terminal, TerminalVis::Hidden);
        assert_eq!(c.reveal, None);
    }

    #[test]
    fn a_resume_from_a_transcript_skips_unknown_line_types() {
        let text = r#"
{"type":"user","message":{"role":"user","content":"Hello"}}
{"type":"file-history-snapshot","messageId":"x"}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Hi there."},{"type":"tool_use","id":"u1","name":"Read","input":{"file_path":"app/src/lead.rs"}}]}}
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"u1","content":"use serde;"}]}}
{"type":"progress","data":{}}
not json
"#;
        let loaded = items_from_transcript(text);
        assert_eq!(loaded.skipped, 3, "{loaded:?}");
        assert_eq!(loaded.items.len(), 3, "{:?}", loaded.items);
        assert_eq!(owner_text(&loaded.items[0]), "Hello");
        assert_eq!(assistant(&loaded.items[1]), ("Hi there.", true));
        assert_eq!(
            tool(&loaded.items[2]),
            ("Read", "Read app/src/lead.rs", ToolState::Done, Some("use serde;"))
        );
    }

    #[test]
    fn history_replaces_the_items() {
        let items = vec![Item::Owner {
            id: "owner-1".into(),
            text: "Hello".into(),
        }];
        let c = fold(
            Conversation::default(),
            &Event::History {
                items: items.clone(),
            },
        );
        assert_eq!(c.items, items);
        assert!(c.version > 0);
    }

    #[test]
    fn an_unreadable_transcript_is_a_notice() {
        let c = fold(Conversation::default(), &Event::HistoryUnread);
        assert_eq!(notice(&c.items[0]), "Couldn't read the conversation history.");
    }

    #[test]
    fn a_nonzero_auth_status_starts_with_the_terminal_shown_for_login() {
        assert_eq!(
            reveal_after_auth_status(1),
            (TerminalVis::Shown, Some(Reveal::Login))
        );
        assert_eq!(reveal_after_auth_status(0), (TerminalVis::Hidden, None));
    }

    #[test]
    fn authentication_failed_reveals_the_terminal_for_login() {
        let c = fold(
            Conversation::default(),
            &Event::StopFailure {
                matcher: Some("authentication_failed".into()),
            },
        );
        assert_eq!(c.terminal, TerminalVis::Shown);
        assert_eq!(c.reveal, Some(Reveal::Login));
        assert_eq!(c.reveal_why.as_deref(), Some("Claude needs you to sign in."));
        assert_eq!(notice(&c.items[0]), "The turn stopped: authentication failed.");
    }

    #[test]
    fn elicitation_reveals_the_terminal() {
        let c = fold(Conversation::default(), &Event::Elicitation);
        assert_eq!(c.terminal, TerminalVis::Shown);
        assert_eq!(c.reveal, Some(Reveal::Auto));
    }

    #[test]
    fn notification_elicitation_dialog_reveals_the_terminal() {
        let c = fold(
            Conversation::default(),
            &Event::Notification {
                kind: "elicitation_dialog".into(),
            },
        );
        assert_eq!((c.terminal, c.reveal), (TerminalVis::Shown, Some(Reveal::Auto)));
    }

    #[test]
    fn notification_elicitation_url_dialog_reveals_the_terminal() {
        let c = fold(
            Conversation::default(),
            &Event::Notification {
                kind: "elicitation_url_dialog".into(),
            },
        );
        assert_eq!((c.terminal, c.reveal), (TerminalVis::Shown, Some(Reveal::Auto)));
    }

    #[test]
    fn notification_agent_needs_input_reveals_the_terminal() {
        let c = fold(
            Conversation::default(),
            &Event::Notification {
                kind: "agent_needs_input".into(),
            },
        );
        assert_eq!((c.terminal, c.reveal), (TerminalVis::Shown, Some(Reveal::Auto)));
    }

    #[test]
    fn notification_quota_auto_resume_stale_reveals_the_terminal() {
        let c = fold(
            Conversation::default(),
            &Event::Notification {
                kind: "quota_auto_resume_stale".into(),
            },
        );
        assert_eq!((c.terminal, c.reveal), (TerminalVis::Shown, Some(Reveal::Auto)));
    }

    #[test]
    fn a_bare_picker_command_reveals_the_terminal() {
        let c = fold(
            Conversation::default(),
            &Event::UserPromptSubmit {
                prompt: "/model".into(),
            },
        );
        assert_eq!(c.terminal, TerminalVis::Shown);
        assert_eq!(c.reveal, Some(Reveal::Auto));
        assert_eq!(
            c.reveal_why.as_deref(),
            Some("Claude is showing its /model menu.")
        );
        assert_eq!(owner_text(&c.items[0]), "/model");
    }

    #[test]
    fn a_picker_command_with_an_argument_does_not_reveal() {
        let c = fold(
            Conversation::default(),
            &Event::UserPromptSubmit {
                prompt: "/model opus".into(),
            },
        );
        assert_eq!(c.terminal, TerminalVis::Hidden);
        assert_eq!(c.reveal, None);
    }

    #[test]
    fn permission_plan_and_question_hooks_do_not_become_conversation_events() {
        assert_eq!(
            event_from_hook("PermissionRequest", &json!({"tool_name": "Bash"})),
            None
        );
        assert_eq!(
            event_from_hook(
                "PermissionRequest",
                &json!({"tool_name": "ExitPlanMode"})
            ),
            None
        );
        assert_eq!(
            event_from_hook(
                "PreToolUse",
                &json!({"tool_name": "AskUserQuestion", "tool_input": {}})
            ),
            None
        );
    }

    #[test]
    fn auth_success_hides_a_login_reveal() {
        let mut c = fold(
            Conversation::default(),
            &Event::StopFailure {
                matcher: Some("authentication_failed".into()),
            },
        );
        c = fold(
            c,
            &Event::Notification {
                kind: "auth_success".into(),
            },
        );
        assert_eq!(c.terminal, TerminalVis::Hidden);
        assert_eq!(c.reveal, None);
        assert_eq!(notice(&c.items[1]), "Signed in.");
    }

    #[test]
    fn sending_hides_an_automatic_reveal() {
        let mut c = fold(Conversation::default(), &Event::Elicitation);
        c = fold(
            c,
            &Event::UserPromptSubmit {
                prompt: "go on".into(),
            },
        );
        assert_eq!(c.terminal, TerminalVis::Hidden);
        assert_eq!(c.reveal, None);
        assert_eq!(owner_text(&c.items[0]), "go on");
    }

    #[test]
    fn stop_hides_an_automatic_reveal() {
        let mut c = fold(Conversation::default(), &Event::Elicitation);
        c = fold(c, &Event::Stop);
        assert_eq!(c.terminal, TerminalVis::Hidden);
        assert_eq!(c.reveal, None);
    }

    #[test]
    fn a_manual_reveal_stays_until_the_toggle_hides_it() {
        let mut c = fold(Conversation::default(), &Event::Toggle { show: Some(true) });
        assert_eq!((c.terminal, c.reveal), (TerminalVis::Shown, Some(Reveal::Manual)));
        c = fold(
            c,
            &Event::UserPromptSubmit {
                prompt: "hello".into(),
            },
        );
        assert_eq!((c.terminal, c.reveal), (TerminalVis::Shown, Some(Reveal::Manual)));
        c = fold(c, &Event::Stop);
        assert_eq!((c.terminal, c.reveal), (TerminalVis::Shown, Some(Reveal::Manual)));
        c = fold(c, &Event::Toggle { show: Some(false) });
        assert_eq!((c.terminal, c.reveal), (TerminalVis::Hidden, None));
    }

    #[test]
    fn conversation_from_returns_only_new_items() {
        let mut c = Conversation::default();
        c = fold(
            c,
            &Event::UserPromptSubmit {
                prompt: "one".into(),
            },
        );
        c = fold(
            c,
            &Event::UserPromptSubmit {
                prompt: "two".into(),
            },
        );
        assert_eq!(c.meta().length, 2);
        assert!(c.meta().version > 0);
        assert_eq!(c.items_from(1).len(), 1);
        assert_eq!(owner_text(&c.items_from(1)[0]), "two");
        assert!(c.items_from(2).is_empty());
    }
}
