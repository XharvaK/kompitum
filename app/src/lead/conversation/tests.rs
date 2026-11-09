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
