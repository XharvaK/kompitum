//! Render tests for the bell: the popup from the sidebar's foot, what needs the Owner and the notes waiting.
#![allow(unused_imports)]

use super::*;

fn bell_props() -> Value {
    json!({"overlay": "o1"})
}

/// The bell lists what needs the Owner, in its order: the lead's question, a blocked agent, a ready update.
#[gpui::test]
fn the_bell_lists_what_needs_you(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut values = fixture(false, bell_props(), updates("ready", "current"));
    values.insert("vaultJson".into(), vault_notes(vec![]));
    values.insert(
        "agents".into(),
        json!([{"address": "t1", "pane": "p1", "status": "blocked", "title": "Build"}]),
    );
    values.insert(
        "lead".into(),
        json!({"state": "asking", "ask": {"id": "a1", "terminal": "t1", "kind": "permission", "input": {}}}),
    );
    let tree = describe_settled(cx, &staged, "mods/bell", values, None).unwrap();
    for shown in [
        "bell-lead",
        "bell-agent-t1",
        "bell-update-app",
        "Claude is asking you",
        "Show",
        "Restart",
    ] {
        assert!(tree.contains(shown), "missing {shown}: {tree}");
    }
    assert!(!tree.contains("bell-calm"), "{tree}");
}

/// With nothing waiting the bell says so, and offers the mailbox for the notes it doesn't list.
#[gpui::test]
fn the_bell_is_calm_with_nothing_waiting(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut values = fixture(false, bell_props(), updates("current", "current"));
    values.insert("vaultJson".into(), vault_notes(vec![]));
    let tree = describe_settled(cx, &staged, "mods/bell", values, None).unwrap();
    for shown in ["Nothing needs you.", "No notes waiting.", "bell-mailbox"] {
        assert!(tree.contains(shown), "missing {shown}: {tree}");
    }
}

/// The notes waiting for the Owner are listed by title, with their sender; Open shows one in the mailbox.
#[gpui::test]
fn the_bell_lists_the_notes_waiting(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut values = fixture(false, bell_props(), updates("current", "current"));
    values.insert(
        "vaultJson".into(),
        vault_notes(vec![vault_owner_note(), vault_agent_note()]),
    );
    let tree = describe_settled(cx, &staged, "mods/bell", values, None).unwrap();
    for shown in ["bell-note-0", "bell-note-1", "Please review", "Progress on the build", "Open"] {
        assert!(tree.contains(shown), "missing {shown}: {tree}");
    }
    assert!(!tree.contains("bell-more"), "two notes fit: {tree}");
}

/// A worker that ran out is named with the time it is out until; it has no button to act on.
#[gpui::test]
fn the_bell_names_a_worker_that_ran_out(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut values = fixture(false, bell_props(), updates("current", "current"));
    values.insert("vaultJson".into(), vault_notes(vec![]));
    values.insert(
        "statusJson".into(),
        json!({"tasks": {"review": [], "running": [], "queued": []}, "workers": {"workers": [
            {"key": "go", "label": "opencode Go", "state": "blocked", "until_ms": 4102444800000i64}
        ]}}),
    );
    let tree = describe_settled(cx, &staged, "mods/bell", values, None).unwrap();
    assert!(tree.contains("bell-limit-go"), "{tree}");
    assert!(tree.contains("is out until"), "{tree}");
    assert!(!tree.contains("bell-limit-go-do"), "a worker row has no button: {tree}");
}

/// Once a system joined through Connect, the bell's Notes header offers its thread.
#[gpui::test]
fn the_bell_offers_the_thread_with_the_joined_system(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut values = fixture(false, bell_props(), updates("current", "current"));
    values.get_mut("settings").unwrap()["connect"] = json!({
        "grokbot": { "since": "2026-11-01T00:00:00Z", "agent": "manager", "name": "Grok Bot", "link": "grokbot://" }
    });
    let tree = describe_settled(cx, &staged, "mods/bell", values, None).unwrap();
    for shown in ["bell-thread", "With Manager"] {
        assert!(tree.contains(shown), "missing {shown}: {tree}");
    }
}
