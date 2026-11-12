use super::*;
use crate::lead::*;

#[test]
fn a_status_line_gives_the_model_and_the_usage() {
    let input = json!({
        "session_id": "s-1",
        "model": { "id": "claude-haiku-5-5", "display_name": "Haiku 5.5" },
        "context_window": { "total_input_tokens": 271000, "context_window_size": 1000000, "used_percentage": 27.1 },
        "prompt_cache": { "ttl": "1h", "expires_at": 1794440000, "warm": true },
        "rate_limits": {
            "five_hour": { "used_percentage": 42.4, "resets_at": 1794430000 },
            "seven_day": { "used_percentage": 61.0, "resets_at": 1794800000 }
        }
    });
    let (model, usage) = status_line_facts(&input, 5_000);
    assert_eq!(model, Some(json!({ "id": "claude-haiku-5-5", "name": "Haiku 5.5" })));
    assert_eq!(
        usage,
        json!({
            "context_tokens": 271000,
            "context_window": 1000000,
            "compact_at": 444000,
            "cache_ttl": "1h",
            "cache_expires_at": 1794440000,
            "five_hour": { "used_percentage": 42.4, "resets_at": 1794430000 },
            "seven_day": { "used_percentage": 61.0, "resets_at": 1794800000 },
            "session_id": "s-1",
            "seen_ms": 5000,
        })
    );
}

#[test]
fn a_status_line_without_a_context_window_says_null_for_it() {
    let (model, usage) = status_line_facts(&json!({ "model": { "display_name": "Opus 5.5" } }), 1);
    assert_eq!(model, Some(json!({ "id": null, "name": "Opus 5.5" })));
    assert_eq!(usage["context_tokens"], Value::Null);
    assert_eq!(usage["context_window"], Value::Null);
    assert_eq!(usage["cache_expires_at"], Value::Null);
    assert_eq!(usage["five_hour"], Value::Null);
    assert_eq!(usage["compact_at"], json!(444_000));
}

#[test]
fn a_status_line_rounds_the_tokens_from_its_percentage_when_no_count_is_given() {
    let input = json!({ "context_window": { "context_window_size": 1000000, "used_percentage": 27.1 } });
    let (model, usage) = status_line_facts(&input, 1);
    assert_eq!(model, None);
    assert_eq!(usage["context_tokens"], json!(271_000));
    assert_eq!(usage["context_window"], json!(1_000_000));
}

#[test]
fn a_status_line_publishes_only_a_change_in_model_or_usage() {
    let input = json!({
        "model": { "id": "claude-haiku-5-5", "display_name": "Haiku 5.5" },
        "context_window": { "total_input_tokens": 271000, "context_window_size": 1000000 },
    });
    let mut session = Session::default();
    assert!(apply_status_line(&mut session, &input, 1));
    assert!(!apply_status_line(&mut session, &input, 2), "only the arrival time moved");
    assert_eq!(session.usage.as_ref().unwrap()["seen_ms"], json!(2));
    let mut moved = input.clone();
    moved["context_window"]["total_input_tokens"] = json!(272000);
    assert!(apply_status_line(&mut session, &moved, 3));
}

#[test]
fn a_compaction_clears_the_context_size_until_the_next_answer() {
    let mut session = Session::default();
    apply_status_line(&mut session, &json!({ "context_window": { "total_input_tokens": 9 } }), 1);
    session.compacting = true;
    assert_eq!(published_session(&session)["compacting"], json!(true));
    compaction_done(&mut session);
    let value = published_session(&session);
    assert_eq!(value["compacting"], json!(false));
    assert_eq!(value["usage"]["context_tokens"], Value::Null);
    assert_eq!(value["model"], Value::Null);
}

#[test]
fn a_status_line_before_claudes_first_answer_knows_no_context_size() {
    let input = json!({ "context_window": { "total_input_tokens": 0, "context_window_size": 1000000.0, "used_percentage": null } });
    let (_, usage) = status_line_facts(&input, 1);
    assert_eq!(usage["context_tokens"], Value::Null);
    assert_eq!(usage["context_window"], json!(1_000_000));
}

#[test]
fn the_lead_compacts_only_while_it_waits_with_no_card_open() {
    assert!(compact_refusal(None).is_some());
    let mut session = Session::default();
    assert!(compact_refusal(Some(&session)).is_some(), "no state yet");
    session.state = Some(State::Working);
    assert!(compact_refusal(Some(&session)).unwrap().contains("working"));
    session.state = Some(State::Waiting);
    assert_eq!(compact_refusal(Some(&session)), None);
    session.compacting = true;
    assert!(compact_refusal(Some(&session)).unwrap().contains("already"));
    session.compacting = false;
    session.ask = Some(Ask {
        id: "a1".into(),
        terminal: "t1".into(),
        kind: AskKind::Permission,
        input: Value::Null,
    });
    assert!(compact_refusal(Some(&session)).unwrap().contains("asking"));
}

#[test]
fn a_compact_command_is_not_a_turn_and_an_asked_for_compaction_ends_waiting() {
    let prompt = |text: &str| json!({ "prompt": text });
    assert_eq!(state_after_hook("UserPromptSubmit", &prompt("hello")), Some(State::Working));
    assert_eq!(state_after_hook("UserPromptSubmit", &prompt("/compact")), None);
    assert_eq!(state_after_hook("Stop", &Value::Null), Some(State::Waiting));
    assert_eq!(state_after_hook("PostCompact", &json!({ "trigger": "manual" })), Some(State::Waiting));
    assert_eq!(state_after_hook("PostCompact", &json!({ "trigger": "auto" })), None);
    assert_eq!(state_after_hook("StatusLine", &Value::Null), None);
}

/// The clock the auto-compaction tests run at: four hours after the epoch, a whole number of seconds.
const NOW: u64 = 4 * 3_600_000;
const MINUTE: u64 = 60_000;

/// A lead that has waited half an hour, with `tokens` in its context and no cache expiry named.
fn waiting_lead(tokens: u64) -> Session {
    Session {
        started: true,
        state: Some(State::Waiting),
        waiting_since_ms: Some(NOW - 30 * MINUTE),
        usage: Some(json!({ "context_tokens": tokens, "cache_expires_at": null })),
        ..Default::default()
    }
}

/// Whether a lead that is due under rule A (340K tokens, waited half an hour) is still due after `change`.
fn due_after(change: impl FnOnce(&mut Session)) -> bool {
    let mut session = waiting_lead(340_000);
    change(&mut session);
    auto_compact_due(&session, NOW, false)
}

#[test]
fn a_waiting_state_keeps_the_time_it_began() {
    let mut session = Session::default();
    assert!(session.set_state(Some(State::Waiting), 100));
    assert!(!session.set_state(Some(State::Waiting), 500));
    assert_eq!(session.waiting_since_ms, Some(100));
}

#[test]
fn working_clears_the_wait_and_lets_the_app_compact_again() {
    let mut session = Session {
        state: Some(State::Waiting),
        waiting_since_ms: Some(100),
        auto_compact_tried: true,
        ..Default::default()
    };
    assert!(session.set_state(Some(State::Working), 500));
    assert_eq!(session.waiting_since_ms, None);
    assert!(!session.auto_compact_tried);
}

#[test]
fn leaving_waiting_for_nothing_clears_the_wait_too() {
    let mut session = Session { state: Some(State::Waiting), waiting_since_ms: Some(100), ..Default::default() };
    assert!(session.set_state(None, 500));
    assert_eq!(session.waiting_since_ms, None);
}

#[test]
fn a_compaction_starts_its_clock_once() {
    let mut session = Session::default();
    session.set_compacting(true, 100);
    session.set_compacting(true, 500);
    assert_eq!(session.compacting_since_ms, Some(100));
    session.set_compacting(false, 900);
    assert_eq!(session.compacting_since_ms, None);
}

#[test]
fn a_due_lead_is_due_under_rule_a_and_each_fact_can_stop_it() {
    assert!(auto_compact_due(&waiting_lead(340_000), NOW, false));
    assert!(!due_after(|s| s.started = false));
    assert!(!due_after(|s| s.state = Some(State::Working)));
    assert!(!due_after(|s| {
        s.ask = Some(Ask {
            id: "a1".into(),
            terminal: "t1".into(),
            kind: AskKind::Permission,
            input: Value::Null,
        })
    }));
    assert!(!due_after(|s| s.compacting = true));
    assert!(!due_after(|s| s.auto_compact_tried = true));
    assert!(!due_after(|s| s.conversation.terminal = TerminalVis::Shown));
    assert!(!auto_compact_due(&waiting_lead(340_000), NOW, true));
    assert!(!due_after(|s| s.usage = Some(json!({ "context_tokens": null }))));
    assert!(!due_after(|s| s.waiting_since_ms = Some(NOW - 9 * MINUTE)));
}

#[test]
fn rule_b_waits_for_the_idle_time_when_claude_names_no_cache_expiry() {
    let at = |tokens: u64, waited: u64| {
        let mut session = waiting_lead(tokens);
        session.waiting_since_ms = Some(NOW - waited);
        auto_compact_due(&session, NOW, false)
    };
    assert!(!at(332_999, 54 * MINUTE));
    assert!(at(332_999, 55 * MINUTE));
}

#[test]
fn rule_b_compacts_in_the_five_minutes_before_the_cache_expires() {
    let with_expiry = |tokens: u64, expires_in_s: i64| {
        let mut session = waiting_lead(tokens);
        let expires = (NOW / 1000) as i64 + expires_in_s;
        session.usage = Some(json!({ "context_tokens": tokens, "cache_expires_at": expires }));
        auto_compact_due(&session, NOW, false)
    };
    assert!(!with_expiry(250_000, 6 * 60));
    assert!(with_expiry(250_000, 4 * 60));
    assert!(!with_expiry(250_000, -10));
    assert!(!with_expiry(221_999, 4 * 60));
}

#[test]
fn a_busy_core_is_one_with_a_running_queued_or_review_task() {
    assert!(tasks_busy(&json!({ "tasks": { "running": [{ "name": "a" }], "queued": [], "review": [] } })));
    assert!(tasks_busy(&json!({ "tasks": { "running": [], "queued": [{ "name": "b" }], "review": [] } })));
    assert!(tasks_busy(&json!({ "tasks": { "running": [], "queued": [], "review": [{ "name": "c" }] } })));
    assert!(!tasks_busy(&json!({ "tasks": { "running": [], "queued": [], "review": [] } })));
    assert!(tasks_busy(&Value::Null));
    assert!(tasks_busy(&json!({})));
}

#[test]
fn a_compaction_past_ten_minutes_is_overdue() {
    let mut session = Session { compacting: true, compacting_since_ms: Some(0), ..Default::default() };
    assert!(!compaction_overdue(&session, 9 * MINUTE));
    assert!(compaction_overdue(&session, 11 * MINUTE));
    session.compacting = false;
    assert!(!compaction_overdue(&session, 11 * MINUTE));
}

#[test]
fn a_menu_in_claudes_terminal_refuses_a_compaction() {
    let mut session = Session { state: Some(State::Waiting), ..Default::default() };
    session.conversation.reveal = Some(Reveal::Auto);
    assert_eq!(
        compact_refusal(Some(&session)),
        Some("Claude is showing a menu in its terminal: answer it first")
    );
}

#[test]
fn a_lead_whose_program_the_core_lost_runs_a_shell_not_claude() {
    let record = |argv: Value, state: &str, pid: Value| {
        json!({ "state": state, "labels": { "lead": "claude" }, "pid": pid, "argv": argv })
    };
    assert!(!lead_program_lost(&record(json!(["C:\\tools\\claude.exe", "--resume", "x"]), "live", json!(42)), "claude", "claude"));
    assert!(lead_program_lost(&record(json!(["C:\\Windows\\System32\\cmd.exe"]), "live", json!(42)), "claude", "claude"));
    assert!(!lead_program_lost(&record(json!(["cmd.exe"]), "finished", json!(42)), "claude", "claude"));
    assert!(!lead_program_lost(&record(json!(["cmd.exe"]), "live", Value::Null), "claude", "claude"));
    assert!(!lead_program_lost(&record(json!(["cmd.exe"]), "live", json!(42)), "pi", "claude"));
    assert!(!lead_program_lost(&record(json!(["cmd.exe"]), "live", json!(42)), "claude", ""));
    // Claude started again by hand in the core's shell is the Owner's: it is left alone.
    let mut by_hand = record(json!(["powershell.exe", "-NoExit"]), "live", json!(42));
    by_hand["foreground"] = json!({ "name": "node.exe", "argv": ["node.exe", "C:/npm/claude-code/cli.js"] });
    assert!(!lead_program_lost(&by_hand, "claude", "claude"));
    by_hand["foreground"] = json!({ "name": "powershell.exe", "argv": ["powershell.exe", "-NoExit"] });
    assert!(lead_program_lost(&by_hand, "claude", "claude"));
}
