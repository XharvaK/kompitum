use super::*;
use crate::lead::{apply_startup_reveal, hide_startup};
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

fn steering(item: &Item) -> bool {
    match item {
        Item::Owner { steering, .. } => *steering,
        _ => panic!("expected owner, got {item:?}"),
    }
}

fn edit(item: &Item) -> Option<&EditStat> {
    match item {
        Item::Tool { edit, .. } => edit.as_ref(),
        _ => panic!("expected tool, got {item:?}"),
    }
}

#[test]
fn the_apps_own_model_command_is_not_drawn_as_the_owners_words() {
    let mut c = Conversation::default();
    c.app_sent.push("/model opus".into());
    c = fold(
        c,
        &Event::UserPromptSubmit {
            prompt: "/model opus".into(),
        },
    );
    assert!(c.items.is_empty(), "{:?}", c.items);
    assert!(c.app_sent.is_empty(), "the echo consumes the app's command");
}

#[test]
fn poll_events_make_no_owner_item_and_no_new_turn() {
    let poll = event_from_hook(
        "UserPromptSubmit",
        &json!({"prompt": "the code printed this", "source": "poll_event"}),
    );
    assert!(poll.is_none(), "a poll event is the code's output, not the Owner's words");
    let mut c = fold(
        Conversation::default(),
        &Event::UserPromptSubmit {
            prompt: "the real question".into(),
        },
    );
    if let Some(event) = poll {
        c = fold(c, &event);
    }
    assert_eq!(c.items.len(), 1, "{:?}", c.items);
    assert_eq!(owner_text(&c.items[0]), "the real question");
}

#[test]
fn a_repl_call_draws_a_code_row_and_keeps_its_code() {
    let c = fold(
        Conversation::default(),
        &Event::PreToolUse {
            id: "r1".into(),
            name: "REPL".into(),
            input: json!({"code": "const x = 1;\nreturn x;"}),
        },
    );
    match &c.items[0] {
        Item::Tool {
            name,
            summary,
            state,
            code,
            ..
        } => {
            assert_eq!(name, "REPL");
            assert_eq!(summary, "Code");
            assert_eq!(*state, ToolState::Running);
            assert_eq!(code.as_deref(), Some("const x = 1;\nreturn x;"));
        }
        other => panic!("expected a tool, got {other:?}"),
    }
    let c = fold(
        c,
        &Event::PostToolUse {
            id: "r1".into(),
            name: "REPL".into(),
            input: json!({"code": "const x = 1;\nreturn x;"}),
            result: "1".into(),
            response: Value::Null,
        },
    );
    match &c.items[0] {
        Item::Tool {
            summary,
            state,
            result,
            code,
            ..
        } => {
            assert_eq!(summary, "Code");
            assert_eq!(*state, ToolState::Done);
            assert_eq!(result.as_deref(), Some("1"));
            assert_eq!(code.as_deref(), Some("const x = 1;\nreturn x;"));
        }
        other => panic!("expected a tool, got {other:?}"),
    }
}

#[test]
fn a_transcript_with_only_a_repl_call_draws_the_code_row_alone() {
    let text = r#"{"type":"user","message":{"role":"user","content":"Run it"}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"r1","name":"REPL","input":{"code":"return 42"}}]}}
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"r1","content":"42"}]}}
"#;
    let loaded = items_from_transcript(text);
    assert_eq!(loaded.items.len(), 2, "{:?}", loaded.items);
    assert_eq!(owner_text(&loaded.items[0]), "Run it");
    match &loaded.items[1] {
        Item::Tool {
            summary,
            code,
            result,
            ..
        } => {
            assert_eq!(summary, "Code");
            assert_eq!(code.as_deref(), Some("return 42"));
            assert_eq!(result.as_deref(), Some("42"));
        }
        other => panic!("expected the code row, got {other:?}"),
    }
}

#[test]
fn a_btw_question_is_drawn_and_shows_the_terminal() {
    let c = fold(
        Conversation::default(),
        &Event::UserPromptSubmit {
            prompt: "/btw what is x".into(),
        },
    );
    assert_eq!(owner_text(&c.items[0]), "/btw what is x");
    assert_eq!(c.reveal, Some(Reveal::Auto));
    assert_eq!(
        c.reveal_why.as_deref(),
        Some("Claude is answering a side question in its terminal.")
    );
}

#[test]
fn a_word_beginning_with_btw_is_not_the_command() {
    let c = fold(
        Conversation::default(),
        &Event::UserPromptSubmit {
            prompt: "/btwx".into(),
        },
    );
    assert_eq!(c.reveal, None);
}

#[test]
fn an_ordinary_prompt_hides_the_btw_reveal() {
    let c = fold(
        Conversation::default(),
        &Event::UserPromptSubmit {
            prompt: "/btw what is x".into(),
        },
    );
    let c = fold(
        c,
        &Event::UserPromptSubmit {
            prompt: "and now the real work".into(),
        },
    );
    assert_eq!(c.terminal, TerminalVis::Hidden);
    assert_eq!(c.reveal, None);
}

#[test]
fn the_same_words_typed_by_the_owner_are_drawn() {
    let c = fold(
        Conversation::default(),
        &Event::UserPromptSubmit {
            prompt: "/model opus".into(),
        },
    );
    assert_eq!(owner_text(&c.items[0]), "/model opus");
}

#[test]
fn two_commands_the_app_sent_are_both_dropped() {
    let mut c = Conversation {
        app_sent: vec!["/model opus".into(), "/effort high".into()],
        ..Default::default()
    };
    c = fold(
        c,
        &Event::UserPromptSubmit {
            prompt: "/model opus".into(),
        },
    );
    c = fold(
        c,
        &Event::UserPromptSubmit {
            prompt: "/effort high".into(),
        },
    );
    assert!(c.items.is_empty(), "{:?}", c.items);
    assert!(c.app_sent.is_empty());
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
            message_id: "m1".into(),
            index: 0,
            last: false,
            delta: "I'll read".into(),
        },
    );
    c = fold(
        c,
        &Event::MessageDisplay {
            message_id: "m1".into(),
            index: 1,
            last: false,
            delta: " the file.\n".into(),
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
            input: json!({"file_path": "app/src/lead.rs"}),
            result: "pub fn fold".into(),
            response: Value::Null,
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
            input: json!({"command": "cargo test"}),
            result: "ok\n".repeat(20),
            response: Value::Null,
        },
    );
    c = fold(
        c,
        &Event::MessageDisplay {
            message_id: "m2".into(),
            index: 0,
            last: true,
            delta: "Done.".into(),
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
        &Event::PostToolUseFailure {
            id: "u1".into(),
            name: "Bash".into(),
            input: json!({"command": "false"}),
            result: "Exit code 1".into(),
        },
    );
    assert_eq!(
        tool(&c.items[0]),
        ("Bash", "Ran `false`", ToolState::Failed, Some("Exit code 1"))
    );
}

#[test]
fn a_running_bash_row_says_it_is_running_its_command() {
    let c = fold(
        Conversation::default(),
        &Event::PreToolUse {
            id: "u1".into(),
            name: "Bash".into(),
            input: json!({"command": "git --version"}),
        },
    );
    assert_eq!(
        tool(&c.items[0]),
        ("Bash", "Running `git --version`", ToolState::Running, None)
    );
}

#[test]
fn a_bash_row_says_ran_once_its_command_has_ended() {
    let mut c = Conversation::default();
    c = fold(
        c,
        &Event::PreToolUse {
            id: "u1".into(),
            name: "Bash".into(),
            input: json!({"command": "git --version"}),
        },
    );
    c = fold(
        c,
        &Event::PostToolUse {
            id: "u1".into(),
            name: "Bash".into(),
            input: json!({"command": "git --version"}),
            result: "git version 2.45.0".into(),
            response: Value::Null,
        },
    );
    assert_eq!(
        tool(&c.items[0]),
        ("Bash", "Ran `git --version`", ToolState::Done, Some("git version 2.45.0"))
    );
}

#[test]
fn a_running_row_that_ends_without_input_turns_its_words_into_the_past() {
    let mut c = Conversation::default();
    c = fold(
        c,
        &Event::PreToolUse {
            id: "u1".into(),
            name: "Bash".into(),
            input: json!({"command": "git --version"}),
        },
    );
    c = fold(
        c,
        &Event::PostToolUse {
            id: "u1".into(),
            name: "Bash".into(),
            input: json!({}),
            result: String::new(),
            response: Value::Null,
        },
    );
    assert_eq!(tool(&c.items[0]).1, "Ran `git --version`");
}

#[test]
fn stop_failure_names_its_error_in_a_notice() {
    let c = fold(
        Conversation::default(),
        &Event::StopFailure {
            error: Some("rate_limit".into()),
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
        steering: false,
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
fn apply_startup_reveal_shows_the_terminal_with_its_reason() {
    let mut c = Conversation::default();
    apply_startup_reveal(&mut c);
    assert_eq!(c.terminal, TerminalVis::Shown);
    assert_eq!(c.reveal, Some(Reveal::Startup));
    assert_eq!(
        c.reveal_why.as_deref(),
        Some("Claude is asking something before it starts.")
    );
}

#[test]
fn hide_startup_clears_the_startup_reveal() {
    let mut c = Conversation::default();
    apply_startup_reveal(&mut c);
    hide_startup(&mut c);
    assert_eq!(c.terminal, TerminalVis::Hidden);
    assert_eq!(c.reveal, None);
    assert_eq!(c.reveal_why, None);
}

#[test]
fn hide_startup_does_not_clear_a_login_reveal() {
    let mut c = fold(
        Conversation::default(),
        &Event::StopFailure {
            error: Some("authentication_failed".into()),
        },
    );
    hide_startup(&mut c);
    assert_eq!(c.terminal, TerminalVis::Shown);
    assert_eq!(c.reveal, Some(Reveal::Login));
    assert_eq!(c.reveal_why.as_deref(), Some("Claude needs you to sign in."));
}

#[test]
fn authentication_failed_reveals_the_terminal_for_login() {
    let c = fold(
        Conversation::default(),
        &Event::StopFailure {
            error: Some("authentication_failed".into()),
        },
    );
    assert_eq!(c.terminal, TerminalVis::Shown);
    assert_eq!(c.reveal, Some(Reveal::Login));
    assert_eq!(c.reveal_why.as_deref(), Some("Claude needs you to sign in."));
    assert_eq!(notice(&c.items[0]), "The turn stopped: sign-in failed.");
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
            error: Some("authentication_failed".into()),
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

fn batch(message_id: &str, index: u64, last: bool, delta: &str) -> Event {
    Event::MessageDisplay {
        message_id: message_id.into(),
        index,
        last,
        delta: delta.into(),
    }
}

#[test]
fn batches_that_arrive_out_of_order_are_assembled_by_index() {
    let mut c = Conversation::default();
    c = fold(c, &batch("m1", 1, false, "B"));
    assert_eq!(assistant(&c.items[0]), ("", false));
    c = fold(c, &batch("m1", 0, false, "A"));
    assert_eq!(assistant(&c.items[0]), ("AB", false));
    c = fold(c, &batch("m1", 2, true, "C"));
    assert_eq!(c.items.len(), 1, "{:?}", c.items);
    assert_eq!(assistant(&c.items[0]), ("ABC", true));
}

#[test]
fn two_message_ids_in_one_turn_are_two_items() {
    let mut c = Conversation::default();
    c = fold(c, &batch("m1", 0, true, "One."));
    c = fold(c, &batch("m2", 0, true, "Two."));
    assert_eq!(c.items.len(), 2, "{:?}", c.items);
    assert_eq!(assistant(&c.items[0]), ("One.", true));
    assert_eq!(assistant(&c.items[1]), ("Two.", true));
}

#[test]
fn a_final_batch_with_an_empty_delta_ends_the_message() {
    let mut c = Conversation::default();
    c = fold(c, &batch("m1", 0, false, "Hi"));
    assert_eq!(assistant(&c.items[0]), ("Hi", false));
    c = fold(c, &batch("m1", 1, true, ""));
    assert_eq!(assistant(&c.items[0]), ("Hi", true));
}

#[test]
fn a_stop_ends_a_message_whose_last_batch_has_not_arrived() {
    let mut c = fold(Conversation::default(), &batch("m1", 0, false, "Hi"));
    c = fold(c, &Event::Stop);
    assert_eq!(assistant(&c.items[0]), ("Hi", true));
    c = fold(c, &batch("m1", 1, true, " there."));
    assert_eq!(c.items.len(), 1, "{:?}", c.items);
    assert_eq!(assistant(&c.items[0]), ("Hi there.", true));
}

#[test]
fn a_pre_that_arrives_after_its_post_does_not_run_the_tool_again() {
    let mut c = Conversation::default();
    c = fold(
        c,
        &Event::PostToolUse {
            id: "u1".into(),
            name: "Read".into(),
            input: json!({"file_path": "a.rs"}),
            result: "one".into(),
            response: Value::Null,
        },
    );
    c = fold(
        c,
        &Event::PreToolUse {
            id: "u1".into(),
            name: "Read".into(),
            input: json!({"file_path": "a.rs"}),
        },
    );
    assert_eq!(c.items.len(), 1, "{:?}", c.items);
    assert_eq!(tool(&c.items[0]), ("Read", "Read a.rs", ToolState::Done, Some("one")));
}

#[test]
fn a_failure_that_arrives_before_its_pre_creates_the_failed_tool() {
    let mut c = Conversation::default();
    c = fold(
        c,
        &Event::PostToolUseFailure {
            id: "u1".into(),
            name: "Bash".into(),
            input: json!({"command": "false"}),
            result: "Exit code 1".into(),
        },
    );
    c = fold(
        c,
        &Event::PreToolUse {
            id: "u1".into(),
            name: "Bash".into(),
            input: json!({"command": "false"}),
        },
    );
    assert_eq!(c.items.len(), 1, "{:?}", c.items);
    assert_eq!(
        tool(&c.items[0]),
        ("Bash", "Ran `false`", ToolState::Failed, Some("Exit code 1"))
    );
}

#[test]
fn a_post_with_no_id_finishes_the_running_tool_of_its_name() {
    let mut c = fold(
        Conversation::default(),
        &Event::PreToolUse {
            id: String::new(),
            name: "Bash".into(),
            input: json!({"command": "ls"}),
        },
    );
    c = fold(
        c,
        &Event::PostToolUse {
            id: String::new(),
            name: "Bash".into(),
            input: json!({"command": "ls"}),
            result: "a.rs".into(),
            response: Value::Null,
        },
    );
    assert_eq!(c.items.len(), 1, "{:?}", c.items);
    assert_eq!(tool(&c.items[0]), ("Bash", "Ran `ls`", ToolState::Done, Some("a.rs")));
}

#[test]
fn a_post_failure_hook_names_the_error_and_keeps_twelve_lines() {
    let error = (1..=20).map(|n| format!("line {n}")).collect::<Vec<_>>().join("\n");
    let event = event_from_hook(
        "PostToolUseFailure",
        &json!({"tool_name": "Bash", "tool_use_id": "u1", "tool_input": {"command": "x"}, "error": error}),
    );
    let c = fold(Conversation::default(), event.as_ref().unwrap());
    let (_, _, state, result) = tool(&c.items[0]);
    assert_eq!(state, ToolState::Failed);
    assert_eq!(result.unwrap().lines().count(), 12);
    assert!(result.unwrap().starts_with("line 1\n"));
}

#[test]
fn an_interrupted_tool_failure_says_stopped() {
    let event = event_from_hook(
        "PostToolUseFailure",
        &json!({"tool_name": "Bash", "tool_use_id": "u1", "tool_input": {"command": "sleep 9"}, "error": "Interrupted", "is_interrupt": true}),
    );
    let c = fold(Conversation::default(), event.as_ref().unwrap());
    assert_eq!(tool(&c.items[0]).2, ToolState::Failed);
    assert_eq!(tool(&c.items[0]).3, Some("Stopped."));
}

#[test]
fn a_bash_response_shows_its_stdout_and_stderr() {
    let event = event_from_hook(
        "PostToolUse",
        &json!({"tool_name": "Bash", "tool_use_id": "u1", "tool_input": {"command": "x"},
                "tool_response": {"stdout": "ok\n", "stderr": "warn\n"}}),
    );
    assert_eq!(
        event,
        Some(Event::PostToolUse {
            id: "u1".into(),
            name: "Bash".into(),
            input: json!({"command": "x"}),
            result: "ok\nwarn".into(),
            response: json!({"stdout": "ok\n", "stderr": "warn\n"}),
        })
    );
}

#[test]
fn a_structured_response_with_no_text_is_an_empty_result_not_json() {
    let event = event_from_hook(
        "PostToolUse",
        &json!({"tool_name": "Write", "tool_use_id": "u1", "tool_input": {"file_path": "a.rs"},
                "tool_response": {"filePath": "a.rs", "type": "create"}}),
    );
    let Some(Event::PostToolUse { result, .. }) = event else {
        panic!("{event:?}");
    };
    assert_eq!(result, "");
}

#[test]
fn a_string_response_and_text_parts_are_the_result() {
    let text = event_from_hook(
        "PostToolUse",
        &json!({"tool_name": "Grep", "tool_use_id": "u1", "tool_input": {}, "tool_response": "a.rs:1"}),
    );
    assert!(matches!(text, Some(Event::PostToolUse { ref result, .. }) if result == "a.rs:1"), "{text:?}");
    let parts = event_from_hook(
        "PostToolUse",
        &json!({"tool_name": "Mcp", "tool_use_id": "u2", "tool_input": {},
                "tool_response": {"content": [{"type": "text", "text": "one"}, {"type": "image"}, {"type": "text", "text": "two"}]}}),
    );
    assert!(matches!(parts, Some(Event::PostToolUse { ref result, .. }) if result == "one\ntwo"), "{parts:?}");
}

#[test]
fn stop_failure_words_each_error_it_names() {
    let cases = [
        ("rate_limit", "The turn stopped: rate limit."),
        ("overloaded", "The turn stopped: Claude is overloaded."),
        ("billing_error", "The turn stopped: a billing problem."),
        ("max_output_tokens", "The turn stopped: the reply hit its length limit."),
        ("oauth_org_not_allowed", "The turn stopped: oauth org not allowed."),
    ];
    for (error, want) in cases {
        let event = event_from_hook("StopFailure", &json!({"error": error})).unwrap();
        let c = fold(Conversation::default(), &event);
        assert_eq!(notice(&c.items[0]), want, "{error}");
    }
}

#[test]
fn stop_failure_reads_only_the_error_field() {
    let event = event_from_hook("StopFailure", &json!({"matcher": "rate_limit"})).unwrap();
    let c = fold(Conversation::default(), &event);
    assert_eq!(notice(&c.items[0]), "The turn stopped.");
}

#[test]
fn an_auth_storage_failure_keeps_the_login_reveal_and_says_so() {
    let mut c = fold(
        Conversation::default(),
        &Event::StopFailure {
            error: Some("authentication_failed".into()),
        },
    );
    c = fold(
        c,
        &Event::Notification {
            kind: "auth_storage_failure".into(),
        },
    );
    assert_eq!((c.terminal, c.reveal), (TerminalVis::Shown, Some(Reveal::Login)));
    assert_eq!(notice(&c.items[1]), "Signed in, but the sign-in couldn't be saved.");
}

#[test]
fn notification_reads_only_its_notification_type() {
    let event = event_from_hook("Notification", &json!({"matcher": "auth_success"})).unwrap();
    let c = fold(Conversation::default(), &event);
    assert!(c.items.is_empty(), "{:?}", c.items);
}

fn steer(text: &str) -> Event {
    Event::Steer { text: text.into() }
}

#[test]
fn a_steered_message_is_one_item_after_its_hook_and_no_longer_steering() {
    let mut c = fold(Conversation::default(), &steer("look at the tests"));
    assert_eq!(c.items.len(), 1);
    assert!(steering(&c.items[0]));
    c = fold(
        c,
        &Event::UserPromptSubmit {
            prompt: "look at the tests".into(),
        },
    );
    assert_eq!(c.items.len(), 1, "{:?}", c.items);
    assert!(!steering(&c.items[0]));
    assert!(c.steered.is_empty());
}

#[test]
fn a_stop_clears_the_steering_mark() {
    let c = fold(Conversation::default(), &steer("look at the tests"));
    let c = fold(c, &Event::Stop);
    assert!(!steering(&c.items[0]));
}

#[test]
fn a_hook_after_a_stop_still_gives_one_item() {
    let c = fold(Conversation::default(), &steer("look at the tests"));
    let c = fold(c, &Event::Stop);
    let c = fold(
        c,
        &Event::UserPromptSubmit {
            prompt: "look at the tests".into(),
        },
    );
    assert_eq!(c.items.len(), 1, "{:?}", c.items);
}

#[test]
fn a_different_prompt_adds_its_own_item() {
    let c = fold(Conversation::default(), &steer("look at the tests"));
    let c = fold(
        c,
        &Event::UserPromptSubmit {
            prompt: "and then the docs".into(),
        },
    );
    assert_eq!(c.items.len(), 2, "{:?}", c.items);
    assert!(steering(&c.items[0]));
}

#[test]
fn a_steered_entry_is_forgotten_after_two_stops() {
    let mut c = fold(Conversation::default(), &steer("look at the tests"));
    c = fold(c, &Event::Stop);
    c = fold(c, &Event::Stop);
    assert!(c.steered.is_empty(), "{:?}", c.steered);
    c = fold(
        c,
        &Event::UserPromptSubmit {
            prompt: "look at the tests".into(),
        },
    );
    assert_eq!(c.items.len(), 2, "{:?}", c.items);
}

fn hook(name: &str, input: Value, response: Value) -> Event {
    Event::PostToolUse {
        id: format!("{name}-1"),
        name: name.into(),
        input,
        result: String::new(),
        response,
    }
}

#[test]
fn todowrite_replaces_the_task_list() {
    let c = fold(
        Conversation::default(),
        &hook(
            "TodoWrite",
            json!({"todos": [
                {"content": "Read the brief", "status": "completed", "activeForm": "Reading the brief"},
                {"content": "Write the tests", "status": "in_progress", "activeForm": "Writing the tests"}
            ]}),
            Value::Null,
        ),
    );
    assert_eq!(
        c.tasks,
        vec![
            TaskRow {
                id: "1".into(),
                subject: "Read the brief".into(),
                active: "Reading the brief".into(),
                status: "completed".into()
            },
            TaskRow {
                id: "2".into(),
                subject: "Write the tests".into(),
                active: "Writing the tests".into(),
                status: "in_progress".into()
            }
        ]
    );
}

#[test]
fn taskcreate_appends_a_row_with_its_response_id_or_the_next_number() {
    let c = fold(
        Conversation::default(),
        &hook(
            "TaskCreate",
            json!({"subject": "Write the code", "description": "The strip", "activeForm": "Writing the code"}),
            json!({"task": {"id": "7"}}),
        ),
    );
    assert_eq!(
        c.tasks,
        vec![TaskRow {
            id: "7".into(),
            subject: "Write the code".into(),
            active: "Writing the code".into(),
            status: "pending".into()
        }]
    );
    let c = fold(
        c,
        &hook(
            "TaskCreate",
            json!({"subject": "Check it", "activeForm": "Checking it"}),
            Value::Null,
        ),
    );
    assert_eq!(c.tasks[1].id, "2");
}

#[test]
fn taskupdate_changes_its_row() {
    let mut c = fold(
        Conversation::default(),
        &hook(
            "TodoWrite",
            json!({"todos": [{"content": "Write the code", "status": "pending", "activeForm": "Writing the code"}]}),
            Value::Null,
        ),
    );
    c = fold(
        c,
        &hook(
            "TaskUpdate",
            json!({"taskId": "1", "status": "in_progress", "subject": "Write the strip", "activeForm": "Writing the strip"}),
            Value::Null,
        ),
    );
    assert_eq!(c.tasks[0].status, "in_progress");
    assert_eq!(c.tasks[0].subject, "Write the strip");
    assert_eq!(c.tasks[0].active, "Writing the strip");
}

#[test]
fn taskupdate_ignores_an_unknown_id() {
    let c = fold(
        Conversation::default(),
        &hook("TaskUpdate", json!({"taskId": "9", "status": "completed"}), Value::Null),
    );
    assert!(c.tasks.is_empty(), "{:?}", c.tasks);
}

#[test]
fn taskupdate_deleted_removes_its_row() {
    let mut c = fold(
        Conversation::default(),
        &hook(
            "TodoWrite",
            json!({"todos": [
                {"content": "One", "status": "pending", "activeForm": ""},
                {"content": "Two", "status": "pending", "activeForm": ""}
            ]}),
            Value::Null,
        ),
    );
    c = fold(
        c,
        &hook("TaskUpdate", json!({"taskId": "1", "status": "deleted"}), Value::Null),
    );
    assert_eq!(c.tasks.len(), 1, "{:?}", c.tasks);
    assert_eq!(c.tasks[0].subject, "Two");
}

#[test]
fn the_transcript_builds_the_same_task_list() {
    let text = r#"
{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"TodoWrite","input":{"todos":[{"content":"Read","status":"completed","activeForm":"Reading"},{"content":"Write","status":"in_progress","activeForm":"Writing"}]}}]}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t2","name":"TaskCreate","input":{"subject":"Test","description":"The tests","activeForm":"Testing"}}]}}
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t2","content":"Created task"}]},"tool_use_result":{"task":{"id":"3"}}}
"#;
    let loaded = items_from_transcript(text);
    assert_eq!(
        loaded.tasks,
        vec![
            TaskRow {
                id: "1".into(),
                subject: "Read".into(),
                active: "Reading".into(),
                status: "completed".into()
            },
            TaskRow {
                id: "2".into(),
                subject: "Write".into(),
                active: "Writing".into(),
                status: "in_progress".into()
            },
            TaskRow {
                id: "3".into(),
                subject: "Test".into(),
                active: "Testing".into(),
                status: "pending".into()
            }
        ]
    );
}

fn edit_of(name: &str, input: Value) -> Option<EditStat> {
    let c = fold(Conversation::default(), &hook(name, input, Value::Null));
    edit(&c.items[0]).cloned()
}

#[test]
fn an_edit_counts_the_lines_of_its_strings() {
    assert_eq!(
        edit_of("Edit", json!({"file_path": "a.rs", "old_string": "one\ntwo", "new_string": "one\nthree\nfour"})),
        Some(EditStat {
            path: "a.rs".into(),
            added: 3,
            removed: 2
        })
    );
}

#[test]
fn a_multiedit_sums_its_edits() {
    assert_eq!(
        edit_of(
            "MultiEdit",
            json!({"file_path": "a.rs", "edits": [
                {"old_string": "one", "new_string": "two\nthree"},
                {"old_string": "a\nb", "new_string": "c"}
            ]})
        ),
        Some(EditStat {
            path: "a.rs".into(),
            added: 3,
            removed: 3
        })
    );
}

#[test]
fn a_write_counts_its_content_and_a_notebook_its_source() {
    assert_eq!(
        edit_of("Write", json!({"file_path": "a.rs", "content": "one\ntwo\n"})),
        Some(EditStat {
            path: "a.rs".into(),
            added: 2,
            removed: 0
        })
    );
    assert_eq!(
        edit_of("NotebookEdit", json!({"notebook_path": "a.ipynb", "new_source": "one\ntwo"})),
        Some(EditStat {
            path: "a.ipynb".into(),
            added: 2,
            removed: 0
        })
    );
}

#[test]
fn a_structured_patch_wins_over_the_edit_strings() {
    let c = fold(
        Conversation::default(),
        &hook(
            "Edit",
            json!({"file_path": "a.rs", "old_string": "one\ntwo", "new_string": "one"}),
            json!({"structuredPatch": [
                {"lines": [" one", "+two", "+three", "-four"]},
                {"lines": ["+five", "-six"]}
            ]}),
        ),
    );
    assert_eq!(
        edit(&c.items[0]),
        Some(&EditStat {
            path: "a.rs".into(),
            added: 3,
            removed: 2
        })
    );
}

#[test]
fn the_transcript_edits_come_from_the_input() {
    let text = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"e1","name":"Edit","input":{"file_path":"a.rs","old_string":"one","new_string":"two\nthree"}}]}}
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"e1","content":"ok"}]}}
"#;
    let loaded = items_from_transcript(text);
    assert_eq!(
        edit(&loaded.items[0]),
        Some(&EditStat {
            path: "a.rs".into(),
            added: 2,
            removed: 1
        })
    );
}

/// Windows leads run commands with the PowerShell tool: its rows name the command as Bash's do.
#[test]
fn powershell_rows_name_their_command() {
    let input = json!({"command": "py -c \"print(1)\""});
    assert_eq!(tool_summary("PowerShell", &input, ToolState::Running), "Running `py -c \"print(1)\"`");
    assert_eq!(tool_summary("PowerShell", &input, ToolState::Done), "Ran `py -c \"print(1)\"`");
}
