//! The conversation the panel draws. `fold` applies one hook event; it does no IO.

use crate::strings;
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
    /// The app's own `/model` and `/effort` texts, waiting for Claude's hook to echo them: they are
    /// not the Owner's words, so they are not drawn. Not saved.
    #[serde(skip)]
    pub app_sent: Vec<String>,
    /// The Owner's texts sent while the lead worked, and how many Stops have passed since each.
    /// Not saved.
    #[serde(skip)]
    pub steered: Vec<(String, u32)>,
    /// Claude's task list, from its task tools or the transcript.
    #[serde(default)]
    pub tasks: Vec<TaskRow>,
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
        #[serde(default, skip_serializing_if = "is_false")]
        steering: bool,
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        edit: Option<EditStat>,
        /// A `REPL` call's code, shown in the mono font when the row is open.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        code: Option<String>,
    },
    Notice {
        id: String,
        text: String,
    },
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// What one edit changed in one file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EditStat {
    pub path: String,
    pub added: u32,
    pub removed: u32,
}

/// One row of Claude's task list.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TaskRow {
    pub id: String,
    pub subject: String,
    pub active: String,
    pub status: String,
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
        response: Value,
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
    Steer {
        text: String,
    },
}

/// What reading a transcript produced.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TranscriptLoad {
    pub items: Vec<Item>,
    pub tasks: Vec<TaskRow>,
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

    /// Ends one running tool's row as failed, with `result`; false when no row with that id is running.
    pub fn fail_running_tool(&mut self, id: &str, result: String) -> bool {
        let Some(Item::Tool {
            name,
            summary,
            state,
            result: have_result,
            edit,
            ..
        }) = self.items.iter_mut().find(|item| matches!(item, Item::Tool { id: have, .. } if have == id))
        else {
            return false;
        };
        if *state != ToolState::Running {
            return false;
        }
        let ended = ended_summary(summary, name, &Value::Null, ToolState::Failed);
        *summary = ended;
        *state = ToolState::Failed;
        *have_result = Some(result);
        *edit = None;
        self.touched();
        true
    }
}

/// A turn ended: every row still running ends the way an interrupted tool does.
fn end_running_tools(conversation: &mut Conversation) -> bool {
    end_running_items(&mut conversation.items)
}

/// A prompt after a tool call with no result means that call stopped.
fn stop_tools_before_prompt(items: &mut [Item]) {
    end_running_items(items);
}

fn end_running_items(items: &mut [Item]) -> bool {
    let mut changed = false;
    for item in items {
        if let Item::Tool {
            name,
            summary,
            state,
            result,
            edit,
            ..
        } = item
            && *state == ToolState::Running
        {
            let ended = ended_summary(summary, name, &Value::Null, ToolState::Failed);
            *summary = ended;
            *state = ToolState::Failed;
            *result = Some(strings::text("lead.tool-stopped"));
            *edit = None;
            changed = true;
        }
    }
    changed
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
            response,
        } => on_tool_end(&mut conversation, id, name, input, result, response, ToolState::Done),
        Event::PostToolUseFailure {
            id,
            name,
            input,
            result,
        } => on_tool_end(
            &mut conversation,
            id,
            name,
            input,
            result,
            &Value::Null,
            ToolState::Failed,
        ),
        Event::Stop => on_stop(&mut conversation),
        Event::StopFailure { error } => on_stop_failure(&mut conversation, error.as_deref()),
        Event::Notification { kind } => on_notification(&mut conversation, kind),
        Event::Elicitation => {
            if conversation.reveal != Some(Reveal::Manual) {
                conversation.show(Reveal::Auto, Some(strings::text("lead.needs-input")));
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
                text: strings::text("lead.history-unread"),
            });
            conversation.touched();
        }
        Event::Toggle { show } => on_toggle(&mut conversation, *show),
        Event::Steer { text } => on_steer(&mut conversation, text),
    }
    conversation
}

/// The hook name and its JSON, or None when the event is not for the conversation
/// (a permission, plan or question card, or an event we do not draw).
pub fn event_from_hook(event: &str, input: &Value) -> Option<Event> {
    match event {
        "UserPromptSubmit" => {
            if input.get("source").and_then(Value::as_str) == Some("poll_event") {
                return None;
            }
            Some(Event::UserPromptSubmit {
                prompt: string_field(input, "prompt"),
            })
        }
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
                response: input.get("tool_response").cloned().unwrap_or(Value::Null),
            })
        }
        "PostToolUseFailure" => {
            let name = string_field(input, "tool_name");
            if name == "AskUserQuestion" {
                return None;
            }
            // An interrupt has no error worth showing; the first line of an error is the same text as its result.
            let result = if input.get("is_interrupt").and_then(Value::as_bool).unwrap_or(false) {
                strings::text("lead.tool-stopped")
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
    let mut tasks = Vec::new();
    let mut pending_creates: BTreeMap<String, usize> = BTreeMap::new();
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
            Some("user") => apply_user(&mut items, &mut next, &mut tasks, &mut pending_creates, &value),
            Some("assistant") => apply_assistant(&mut items, &mut next, &mut tasks, &mut pending_creates, &value),
            Some("system") if value.get("subtype").and_then(Value::as_str) == Some("compact_boundary") => {
                let id = value.get("uuid").and_then(Value::as_str).map_or_else(|| next("notice"), str::to_owned);
                items.push(Item::Notice {
                    id,
                    text: strings::text("lead.tidied-up"),
                });
            }
            Some(_) => skipped += 1,
            None => skipped += 1,
        }
    }
    TranscriptLoad { items, tasks, skipped }
}

/// `/compact` sent on its own, or with a note after it: Claude's own command, not the Owner's words.
pub fn is_compact_command(text: &str) -> bool {
    text == "/compact" || text.starts_with("/compact ")
}

fn on_submit(conversation: &mut Conversation, prompt: &str) {
    if is_compact_command(prompt) {
        return;
    }
    // The app's own model and reasoning commands: its words, not the Owner's, and gone once echoed.
    if let Some(at) = conversation.app_sent.iter().position(|sent| sent == prompt) {
        conversation.app_sent.remove(at);
        return;
    }
    // A steering message's hook: the item is already drawn, so it only loses its mark.
    if let Some(at) = conversation
        .steered
        .iter()
        .position(|(text, _)| text.trim() == prompt.trim())
    {
        let (text, _) = conversation.steered.remove(at);
        if let Some(Item::Owner { steering, .. }) = conversation
            .items
            .iter_mut()
            .rev()
            .find(|item| matches!(item, Item::Owner { text: have, .. } if have.trim() == text.trim()))
        {
            *steering = false;
        }
        conversation.touched();
        return;
    }
    let side_question = prompt == "/btw" || prompt.starts_with("/btw ");
    if conversation.reveal == Some(Reveal::Auto) && !is_picker_command(prompt) && !side_question {
        conversation.hide();
    }
    if !prompt.is_empty() {
        let id = conversation.next("owner");
        conversation.items.push(Item::Owner {
            id,
            text: prompt.to_owned(),
            steering: false,
        });
        conversation.touched();
    }
    if side_question && conversation.reveal != Some(Reveal::Manual) {
        conversation.show(
            Reveal::Auto,
            Some(strings::text("lead.side-question")),
        );
    } else if is_picker_command(prompt) && conversation.reveal != Some(Reveal::Manual) {
        let why = strings::text_with("lead.showing-menu", &[("menu", prompt.trim())]);
        conversation.show(Reveal::Auto, Some(why));
    }
}

/// The Owner's message went at once while the lead worked: draw it now, marked, until Claude's hook
/// takes it in.
fn on_steer(conversation: &mut Conversation, text: &str) {
    let id = conversation.next("owner");
    conversation.items.push(Item::Owner {
        id,
        text: text.to_owned(),
        steering: true,
    });
    conversation.steered.push((text.to_owned(), 0));
    conversation.touched();
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
        code: have_code,
        ..
    }) = tool_mut(conversation, id)
    {
        // A Pre that arrives after its Post or Failure must not start the tool again.
        if *state == ToolState::Running {
            *have = name.to_owned();
            *have_summary = tool_summary(name, input, ToolState::Running);
            *have_code = tool_code(name, input);
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
        edit: None,
        code: tool_code(name, input),
    });
    conversation.touched();
}

/// The summary of a tool that has ended: built again from the end's own input when it has one, otherwise the
/// running words turned into the past ("Running `ls`" becomes "Ran `ls`").
fn ended_summary(current: &str, name: &str, input: &Value, state: ToolState) -> String {
    if input
        .as_object()
        .is_some_and(|fields| fields.keys().any(|key| key != "cwd"))
    {
        return tool_summary(name, input, state);
    }
    if let Some(rest) = current.strip_prefix("Editing ") {
        return match rest {
            "files" => strings::text("lead.edited-files"),
            "a file" => strings::text("lead.edited-a-file"),
            _ => match rest
                .strip_suffix(" files")
                .filter(|count| !count.is_empty() && count.chars().all(|ch| ch.is_ascii_digit()))
            {
                Some(count) => strings::text_with("lead.edited-count", &[("count", count)]),
                None => strings::text_with("lead.edited-one", &[("path", rest)]),
            },
        };
    }
    match current.strip_prefix("Running ") {
        Some(rest) => strings::text_with("lead.tool-ran", &[("rest", rest)]),
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
    response: &Value,
    state: ToolState,
) {
    let clipped = clip_result(result);
    let result = if clipped.is_empty() { None } else { Some(clipped) };
    let edit = edit_stat(name, input, response, state);
    if state == ToolState::Done {
        apply_task_tool(&mut conversation.tasks, name, input, response);
    }
    if let Some(Item::Tool {
        name: have_name,
        summary: have_summary,
        state: have,
        result: have_result,
        edit: have_edit,
        code: have_code,
        ..
    }) = tool_mut(conversation, id)
    {
        *have_summary = ended_summary(have_summary, have_name, input, state);
        *have = state;
        *have_result = result;
        *have_edit = edit;
        if let Some(code) = tool_code(name, input) {
            *have_code = Some(code);
        }
        conversation.touched();
        return;
    }
    // Without an id, the running tool of this name is the one that ended.
    if id.is_empty() {
        if let Some(Item::Tool {
            summary: have_summary,
            state: have,
            result: have_result,
            edit: have_edit,
            code: have_code,
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
            *have_edit = edit;
            if let Some(code) = tool_code(name, input) {
                *have_code = Some(code);
            }
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
        edit,
        code: tool_code(name, input),
    });
    conversation.touched();
}

fn on_stop(conversation: &mut Conversation) {
    let mut unsteered = false;
    for item in &mut conversation.items {
        if let Item::Owner { steering, .. } = item
            && *steering
        {
            *steering = false;
            unsteered = true;
        }
    }
    for (_, stops) in &mut conversation.steered {
        *stops += 1;
    }
    conversation.steered.retain(|(_, stops)| *stops < 2);
    let tools = end_running_tools(conversation);
    if finish_assistant(conversation) || unsteered || tools {
        conversation.touched();
    }
    if conversation.reveal == Some(Reveal::Auto) {
        conversation.hide();
    }
}

fn on_stop_failure(conversation: &mut Conversation, error: Option<&str>) {
    let _ = finish_assistant(conversation);
    end_running_tools(conversation);
    let id = conversation.next("notice");
    conversation.items.push(Item::Notice {
        id,
        text: stop_failure_text(error),
    });
    conversation.touched();
    if error == Some("authentication_failed") {
        conversation.show(Reveal::Login, Some(strings::text("lead.sign-in-needed")));
    }
}

/// The notice for a turn that stopped on an error, in the app's own words.
fn stop_failure_text(error: Option<&str>) -> String {
    match error {
        Some("rate_limit") => strings::text("lead.turn-stopped-rate-limit"),
        Some("overloaded") => strings::text("lead.turn-stopped-overloaded"),
        Some("authentication_failed") => strings::text("lead.turn-stopped-sign-in-failed"),
        Some("billing_error") => strings::text("lead.turn-stopped-billing"),
        Some("max_output_tokens") => strings::text("lead.turn-stopped-length"),
        Some(other) if !other.is_empty() => {
            strings::text_with("lead.turn-stopped-other", &[("words", &other.replace('_', " "))])
        }
        _ => strings::text("lead.turn-stopped"),
    }
}

fn on_notification(conversation: &mut Conversation, kind: &str) {
    match kind {
        "auth_success" => {
            let id = conversation.next("notice");
            conversation.items.push(Item::Notice {
                id,
                text: strings::text("lead.signed-in"),
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
                text: strings::text("lead.sign-in-unsaved"),
            });
            conversation.touched();
        }
        "elicitation_dialog" | "elicitation_url_dialog" => {
            if conversation.reveal != Some(Reveal::Manual) {
                conversation.show(Reveal::Auto, Some(strings::text("lead.needs-input")));
            }
        }
        "agent_needs_input" => {
            if conversation.reveal != Some(Reveal::Manual) {
                conversation.show(Reveal::Auto, Some(strings::text("lead.needs-input")));
            }
        }
        "quota_auto_resume_stale" => {
            if conversation.reveal != Some(Reveal::Manual) {
                conversation.show(
                    Reveal::Auto,
                    Some(strings::text("lead.waiting-continue")),
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
    if name == "REPL" {
        return strings::text("composer.code-row");
    }
    let path = |keys: &[&str]| {
        keys.iter()
            .find_map(|key| input.get(*key).and_then(Value::as_str))
            .unwrap_or("")
    };
    let cwd = input.get("cwd").and_then(Value::as_str);
    match name {
        "Read" => {
            let file = path(&["file_path", "path"]);
            if file.is_empty() {
                strings::text("lead.read-file")
            } else {
                let file = short_path(file, cwd);
                strings::text_with("lead.read", &[("file", &file)])
            }
        }
        "Write" | "Edit" | "NotebookEdit" | "MultiEdit" => edited_summary(input, cwd, state),
        "Bash" | "PowerShell" => {
            let command = path(&["command"]);
            let running = state == ToolState::Running;
            match (command.is_empty(), running) {
                (true, true) => strings::text("lead.bash-running"),
                (true, false) => strings::text("lead.bash-ran"),
                (false, true) => strings::text_with("lead.bash-running-command", &[("command", command)]),
                (false, false) => strings::text_with("lead.bash-ran-command", &[("command", command)]),
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
                strings::text("lead.fetch-url")
            } else {
                strings::text_with("lead.fetch", &[("url", url)])
            }
        }
        "WebSearch" => {
            let query = path(&["query"]);
            if query.is_empty() {
                strings::text("lead.search-web")
            } else {
                strings::text_with("lead.search-web-for", &[("query", query)])
            }
        }
        other => other.to_owned(),
    }
}

/// A `REPL` call's code from its input; None for every other tool.
fn tool_code(name: &str, input: &Value) -> Option<String> {
    if name != "REPL" {
        return None;
    }
    input
        .get("code")
        .and_then(Value::as_str)
        .filter(|code| !code.is_empty())
        .map(str::to_owned)
}

/// What a done edit tool changed: the path it wrote, and the lines its response's patch, or its own
/// input, added and removed.
fn edit_stat(name: &str, input: &Value, response: &Value, state: ToolState) -> Option<EditStat> {
    if state != ToolState::Done {
        return None;
    }
    let path = first_string(input, &["file_path", "notebook_path"]);
    if path.is_empty() {
        return None;
    }
    if let Some(hunks) = response
        .get("structuredPatch")
        .and_then(Value::as_array)
        .filter(|hunks| !hunks.is_empty())
    {
        let mut added = 0;
        let mut removed = 0;
        for hunk in hunks {
            for line in hunk.get("lines").and_then(Value::as_array).into_iter().flatten() {
                let Some(line) = line.as_str() else { continue };
                if line.starts_with('+') {
                    added += 1;
                } else if line.starts_with('-') {
                    removed += 1;
                }
            }
        }
        return Some(EditStat { path, added, removed });
    }
    let lines = |value: &Value, key: &str| {
        value
            .get(key)
            .and_then(Value::as_str)
            .map_or(0, |text| text.lines().count() as u32)
    };
    let (added, removed) = match name {
        "Edit" => (lines(input, "new_string"), lines(input, "old_string")),
        "MultiEdit" => {
            let mut added = 0;
            let mut removed = 0;
            for edit in input.get("edits").and_then(Value::as_array).into_iter().flatten() {
                added += lines(edit, "new_string");
                removed += lines(edit, "old_string");
            }
            (added, removed)
        }
        "Write" => (lines(input, "content"), 0),
        "NotebookEdit" => (lines(input, "new_source"), 0),
        _ => return None,
    };
    Some(EditStat { path, added, removed })
}

/// The id a TaskCreate's response gave the new task.
fn task_response_id(response: &Value) -> Option<String> {
    let parsed;
    let response = match response {
        Value::String(text) => {
            parsed = serde_json::from_str(text).unwrap_or(Value::Null);
            &parsed
        }
        other => other,
    };
    response
        .pointer("/task/id")
        .or_else(|| response.get("id"))
        .or_else(|| response.get("taskId"))
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
}

/// The id a TaskUpdate names, with the loose spellings Claude's raw stream can carry.
fn task_input_id(input: &Value) -> Option<String> {
    ["taskId", "id", "task_id"]
        .iter()
        .find_map(|key| input.get(*key).and_then(Value::as_str))
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
}

/// Applies one of Claude Code's task tools to the strip's rows.
#[allow(clippy::ptr_arg)] // It replaces the whole list for TodoWrite, so the Vec is the point.
fn apply_task_tool(tasks: &mut Vec<TaskRow>, name: &str, input: &Value, response: &Value) {
    match name {
        "TodoWrite" => {
            let Some(todos) = input.get("todos").and_then(Value::as_array) else {
                return;
            };
            *tasks = todos
                .iter()
                .enumerate()
                .map(|(index, todo)| TaskRow {
                    id: (index + 1).to_string(),
                    subject: first_string(todo, &["content"]),
                    active: first_string(todo, &["activeForm", "active_form"]),
                    status: first_string(todo, &["status"]),
                })
                .collect();
        }
        "TaskCreate" => {
            tasks.push(TaskRow {
                id: task_response_id(response).unwrap_or_else(|| (tasks.len() + 1).to_string()),
                subject: first_string(input, &["subject"]),
                active: first_string(input, &["activeForm", "active_form"]),
                status: "pending".into(),
            });
        }
        "TaskUpdate" => {
            let Some(id) = task_input_id(input) else {
                return;
            };
            let status = input.get("status").and_then(Value::as_str).unwrap_or("");
            if status == "deleted" {
                tasks.retain(|row| row.id != id);
                return;
            }
            let Some(row) = tasks.iter_mut().find(|row| row.id == id) else {
                return;
            };
            if !status.is_empty() {
                row.status = status.to_owned();
            }
            if let Some(subject) = input.get("subject").and_then(Value::as_str) {
                row.subject = subject.to_owned();
            }
            if let Some(active) = input
                .get("activeForm")
                .or_else(|| input.get("active_form"))
                .and_then(Value::as_str)
            {
                row.active = active.to_owned();
            }
        }
        _ => {}
    }
}

fn edited_summary(input: &Value, cwd: Option<&str>, state: ToolState) -> String {
    let running = state == ToolState::Running;
    if let Some(files) = input.get("files").and_then(Value::as_array) {
        let paths: Vec<&str> = files.iter().filter_map(Value::as_str).collect();
        return match paths.as_slice() {
            [] if running => strings::text("lead.editing-files"),
            [] => strings::text("lead.edited-files"),
            [one] if running => {
                let path = short_path(one, cwd);
                strings::text_with("lead.editing-one", &[("path", &path)])
            }
            [one] => {
                let path = short_path(one, cwd);
                strings::text_with("lead.edited-one", &[("path", &path)])
            }
            many if running => {
                strings::text_with("lead.editing-count", &[("count", &many.len().to_string())])
            }
            many => strings::text_with("lead.edited-count", &[("count", &many.len().to_string())]),
        };
    }
    match input
        .get("file_path")
        .or_else(|| input.get("path"))
        .and_then(Value::as_str)
    {
        Some(path) if !path.is_empty() => {
            let path = short_path(path, cwd);
            if running {
                strings::text_with("lead.editing-one", &[("path", &path)])
            } else {
                strings::text_with("lead.edited-one", &[("path", &path)])
            }
        }
        _ if running => strings::text("lead.editing-a-file"),
        _ => strings::text("lead.edited-a-file"),
    }
}

/// A path as a row reads it: under the lead's folder when it is, and clipped in the middle when
/// it is still longer than 60 characters.
const PATH_LIMIT: usize = 60;

fn short_path(path: &str, cwd: Option<&str>) -> String {
    let path = relative_path(path, cwd);
    if path.chars().count() <= PATH_LIMIT {
        path
    } else {
        clip_path(&path)
    }
}

/// `path` under `cwd` reads without the folder (case-insensitively on Windows, `/` and `\` alike);
/// anything else stays whole.
fn relative_path(path: &str, cwd: Option<&str>) -> String {
    let Some(folder) = cwd.filter(|folder| !folder.is_empty()) else {
        return path.to_owned();
    };
    let folder = folder.trim_end_matches(['/', '\\']);
    let Some(prefix) = path.get(..folder.len()) else {
        return path.to_owned();
    };
    let Some(separator) = path.as_bytes().get(folder.len()) else {
        return path.to_owned();
    };
    if !matches!(separator, b'/' | b'\\') {
        return path.to_owned();
    }
    let folder_key = folder.replace('\\', "/");
    let prefix_key = prefix.replace('\\', "/");
    let inside = if cfg!(windows) {
        prefix_key.eq_ignore_ascii_case(&folder_key)
    } else {
        prefix_key == folder_key
    };
    if inside {
        path[folder.len() + 1..].to_owned()
    } else {
        path.to_owned()
    }
}

/// A long path as its root, `…`, and its last two parts whole.
fn clip_path(path: &str) -> String {
    let mut parts = Vec::new();
    let mut start = 0;
    for (index, ch) in path.char_indices() {
        if matches!(ch, '/' | '\\') {
            if index > start {
                parts.push((start, index));
            }
            start = index + 1;
        }
    }
    if start < path.len() {
        parts.push((start, path.len()));
    }
    if parts.len() <= 2 {
        return path.to_owned();
    }
    let mut root_end = parts[0].1;
    let mut prefix_end = 0;
    if path.as_bytes().get(1) == Some(&b':') {
        prefix_end = if matches!(path.as_bytes().get(2), Some(b'/' | b'\\')) { 3 } else { 2 };
        root_end = parts.get(1).map_or(prefix_end, |part| part.1);
    } else if path.starts_with("\\\\") {
        root_end = parts.get(1).map_or(parts[0].1, |part| part.1);
        prefix_end = parts[0].1;
    } else if path.starts_with('/') {
        prefix_end = 1;
        root_end = parts[0].1;
    }
    let tail_start = parts[parts.len() - 2].0;
    if root_end > tail_start {
        root_end = prefix_end;
    }
    let separator = if path.contains('\\') { '\\' } else { '/' };
    let root = &path[..root_end];
    let tail = &path[tail_start..];
    if root.is_empty() {
        format!("…{separator}{tail}")
    } else if root.ends_with(['/', '\\']) {
        format!("{root}…{separator}{tail}")
    } else {
        format!("{root}{separator}…{separator}{tail}")
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
    let tool = input.get("tool_input").cloned().unwrap_or(Value::Null);
    with_cwd(tool, input.get("cwd").and_then(Value::as_str))
}

/// Carries the hook's `cwd` with a tool's own input, for its row's paths.
fn with_cwd(mut input: Value, cwd: Option<&str>) -> Value {
    if let (Some(cwd), Some(fields)) = (cwd.filter(|cwd| !cwd.is_empty()), input.as_object_mut()) {
        fields.insert("cwd".into(), Value::String(cwd.to_owned()));
    }
    input
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

fn apply_user(
    items: &mut Vec<Item>,
    next: &mut impl FnMut(&str) -> String,
    tasks: &mut [TaskRow],
    pending_creates: &mut BTreeMap<String, usize>,
    value: &Value,
) {
    // A compaction's summary and Claude's meta lines are written by Claude, not the Owner.
    if value["isCompactSummary"] == true || value["isMeta"] == true {
        return;
    }
    match message_content(value) {
        Value::String(text) if !text.is_empty() => {
            stop_tools_before_prompt(items);
            items.push(Item::Owner {
                id: next("owner"),
                text: text.clone(),
                steering: false,
            });
        }
        Value::Array(parts) => {
            let mut texts = Vec::new();
            for part in parts {
                let kind = part.get("type").and_then(Value::as_str).unwrap_or("");
                if kind == "tool_result" {
                    flush_owner(items, next, &mut texts);
                    apply_tool_result(items, part, tasks, pending_creates, value);
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
    stop_tools_before_prompt(items);
    items.push(Item::Owner {
        id: next("owner"),
        text: texts.join(""),
        steering: false,
    });
    texts.clear();
}

#[allow(clippy::ptr_arg)] // A TaskCreate appends to the list.
fn apply_assistant(
    items: &mut Vec<Item>,
    next: &mut impl FnMut(&str) -> String,
    tasks: &mut Vec<TaskRow>,
    pending_creates: &mut BTreeMap<String, usize>,
    value: &Value,
) {
    match message_content(value) {
        Value::String(text) if !text.is_empty() => append_assistant(items, next, text),
        Value::Array(parts) => {
            for part in parts {
                let kind = part.get("type").and_then(Value::as_str).unwrap_or("");
                if kind == "tool_use" {
                    let name = part.get("name").and_then(Value::as_str).unwrap_or("tool");
                    // A question is the card's to answer, never a tool row; the live hook path skips it too.
                    if name == "AskUserQuestion" {
                        continue;
                    }
                    let tool_use_id = part
                        .get("id")
                        .and_then(Value::as_str)
                        .filter(|id| !id.is_empty())
                        .map(str::to_owned);
                    let id = tool_use_id
                        .clone()
                        .unwrap_or_else(|| next("tool"));
                    let input = with_cwd(
                        part.get("input").cloned().unwrap_or(Value::Null),
                        value.get("cwd").and_then(Value::as_str),
                    );
                    if name == "TaskCreate" {
                        let index = tasks.len();
                        tasks.push(TaskRow {
                            id: (index + 1).to_string(),
                            subject: first_string(&input, &["subject"]),
                            active: first_string(&input, &["activeForm", "active_form"]),
                            status: "pending".into(),
                        });
                        if let Some(tool_use_id) = tool_use_id {
                            pending_creates.insert(tool_use_id, index);
                        }
                    } else {
                        apply_task_tool(tasks, name, &input, &Value::Null);
                    }
                    items.push(Item::Tool {
                        id,
                        name: name.to_owned(),
                        summary: tool_summary(name, &input, ToolState::Running),
                        state: ToolState::Running,
                        result: None,
                        edit: edit_stat(name, &input, &Value::Null, ToolState::Done),
                        code: tool_code(name, &input),
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

fn apply_tool_result(
    items: &mut Vec<Item>,
    part: &Value,
    tasks: &mut [TaskRow],
    pending_creates: &mut BTreeMap<String, usize>,
    value: &Value,
) {
    let id = part.get("tool_use_id").and_then(Value::as_str).unwrap_or("");
    let result = match part.get("content") {
        Some(content) => content_text(Some(content)),
        None => part.get("text").and_then(Value::as_str).unwrap_or("").to_owned(),
    };
    let failed = part.get("is_error").and_then(Value::as_bool).unwrap_or(false);
    let state = if failed { ToolState::Failed } else { ToolState::Done };
    let clipped = clip_result(&result);
    if let Some(index) = pending_creates.remove(id) {
        let response = tool_response(value, part);
        if let Some(task_id) = task_response_id(&response)
            && let Some(row) = tasks.get_mut(index)
        {
            row.id = task_id;
        }
    }
    if let Some(Item::Tool {
        name: have_name,
        summary: have_summary,
        state: have,
        result: have_result,
        edit: have_edit,
        ..
    }) = items.iter_mut().find(|item| matches!(item, Item::Tool { id: have, .. } if have == id))
    {
        *have_summary = ended_summary(have_summary, have_name, &Value::Null, state);
        *have = state;
        *have_result = if clipped.is_empty() { None } else { Some(clipped) };
        if failed {
            *have_edit = None;
        }
    }
}

/// The structured response a transcript line carries for its tool result: Claude Code writes a
/// TaskCreate's assigned id in `tool_use_result`, and some lines carry it as the result's JSON.
fn tool_response(value: &Value, part: &Value) -> Value {
    if let Some(response) = value
        .get("tool_use_result")
        .or_else(|| value.get("toolUseResult"))
    {
        return response.clone();
    }
    if let Some(response) = part
        .get("tool_use_result")
        .or_else(|| part.get("toolUseResult"))
    {
        return response.clone();
    }
    match part.get("content") {
        Some(content @ Value::Object(_)) => content.clone(),
        Some(Value::String(text)) => serde_json::from_str(text).unwrap_or(Value::Null),
        _ => Value::Null,
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
