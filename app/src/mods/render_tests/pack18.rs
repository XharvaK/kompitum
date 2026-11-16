//! Pack 18a: the menu card, "You chose" rows, and a bell that counts the lead once.
#![allow(unused_imports)]

use super::*;

fn menu_value() -> Value {
    json!({
        "title": "Select model",
        "body": [
            {"text": "Switch between Claude models.", "mono": false},
            {"text": "function greet() {\n  hi();\n}", "mono": true}
        ],
        "rows": [
            {"label": "Default (recommended)", "detail": "Opus 5.5 · Best for everyday, complex tasks", "current": false},
            {"label": "Haiku 5.5", "detail": "Fastest for quick answers", "current": true},
            {"label": "More models", "detail": "", "current": false}
        ],
        "pointer": 1,
        "notes": [{"text": "◐ Medium effort (default)", "mono": false}],
        "actions": [
            {"key": "enter", "label": "Set as default"},
            {"key": "s", "label": "Use this session only"},
            {"key": "esc", "label": "Cancel"}
        ]
    })
}

fn lead_with_menu(menu: Value) -> Value {
    json!({
        "state": "waiting",
        "ask": null,
        "started": true,
        "terminal": "hidden",
        "terminal_shown": false,
        "reveal": null,
        "reveal_why": null,
        "conversation": {"version": 1, "length": 0},
        "menu": menu,
        "menu_busy": false,
        "needs_you": false
    })
}

fn composer_menu_values(menu: Value) -> BTreeMap<String, Value> {
    let mut values = fixture(true, props(), updates("current", "current"));
    values.insert("knownAgents".into(), agent_catalog());
    values.insert("lead".into(), lead_with_menu(menu));
    values
}

/// A staged menu draws its title, rows and buttons where the ask cards go.
#[gpui::test]
fn the_composer_draws_claudes_menu_as_a_card(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe_settled(
        cx,
        &staged,
        "mods/composer",
        composer_menu_values(menu_value()),
        None,
    )
    .unwrap();
    for shown in [
        "In Claude's terminal",
        "Select model",
        "Switch between Claude models.",
        "Haiku 5.5",
        "Fastest for quick answers",
        "Default (recommended)",
        "More models",
        "◐ Medium effort (default)",
        "composer-menu-row-0",
        "composer-menu-row-1",
        "composer-menu-enter",
        "composer-menu-s",
        "composer-menu-esc",
        "composer-menu-show-terminal",
        "Set as default",
        "Use this session only",
        "Cancel",
        "Show terminal",
    ] {
        assert!(tree.contains(shown), "missing {shown}: {tree}");
    }
}

fn composer_menu_effects(cx: &mut TestAppContext, setup: &str, keys: &str) -> Vec<Vec<Value>> {
    let mut values = composer_menu_values(menu_value());
    values.insert("lead".into(), lead_with_menu(menu_value()));
    composer_run_effects_after_keys(cx, values, keys, Some(setup))
}

#[gpui::test]
fn enter_before_the_menu_cards_wait_does_nothing(cx: &mut TestAppContext) {
    let effects = composer_menu_effects(
        cx,
        "this.menuSince = Date.now() + 60000;",
        "enter",
    );
    assert!(
        effects.iter().all(|args| args.first() != Some(&json!("lead.menu"))),
        "Enter before the wait must press nothing: {effects:?}"
    );
}

#[gpui::test]
fn enter_after_the_menu_cards_wait_presses_the_highlighted_row(cx: &mut TestAppContext) {
    let effects = composer_menu_effects(cx, "this.menuSince = 1;", "enter");
    assert!(
        effects.iter().any(|args| {
            args.first() == Some(&json!("lead.menu"))
                && args.get(1)
                    == Some(&json!({"terminal": "t1", "key": "enter", "row": 1, "label": "Haiku 5.5"}))
        }),
        "Enter presses the menu's highlighted row: {effects:?}"
    );
}

/// A digit highlights that row, then Enter picks it.
#[gpui::test]
fn a_digit_then_enter_picks_that_menu_row(cx: &mut TestAppContext) {
    let effects = composer_menu_effects(cx, "this.menuSince = 1;", "3 enter");
    assert!(
        effects.iter().any(|args| {
            args.first() == Some(&json!("lead.menu"))
                && args.get(1)
                    == Some(&json!({"terminal": "t1", "key": "enter", "row": 2, "label": "More models"}))
        }),
        "digit 3 then Enter picks the third row: {effects:?}"
    );
}

/// An ask always wins over a menu that arrived with it.
#[gpui::test]
fn an_ask_staged_with_a_menu_draws_the_ask(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut values = composer_menu_values(menu_value());
    values.insert(
        "lead".into(),
        json!({
            "state": "asking",
            "ask": {"id": "a1", "terminal": "t1", "kind": "permission", "input": {"tool_name": "Bash", "tool_input": {"command": "ls"}}},
            "started": true,
            "terminal": "hidden",
            "terminal_shown": false,
            "conversation": {"version": 1, "length": 0},
            "menu": menu_value(),
            "menu_busy": false
        }),
    );
    let tree = describe_settled(cx, &staged, "mods/composer", values, None).unwrap();
    assert!(tree.contains("composer-card"), "{tree}");
    assert!(tree.contains("composer-allow"), "{tree}");
    assert!(
        !tree.contains("composer-menu-row-0") && !tree.contains("composer-menu-enter"),
        "the menu card gives way to the ask: {tree}"
    );
}

/// An answered question is drawn quietly: its question, then what the Owner chose or answered.
#[gpui::test]
fn the_composer_draws_an_answered_question(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut values = fixture(true, props(), updates("current", "current"));
    values.insert("knownAgents".into(), agent_catalog());
    values.insert(
        "lead".into(),
        json!({
            "state": "waiting",
            "ask": null,
            "started": true,
            "terminal": "hidden",
            "terminal_shown": false,
            "conversation": {"version": 1, "length": 2},
            "menu": null,
            "menu_busy": false
        }),
    );
    values.insert(
        "conversation".into(),
        json!([{
            "kind": "answer",
            "id": "answer-tu1",
            "rows": [
                {"question": "Rain or sun?", "answer": "Rain", "chosen": true},
                {"question": "Anything else?", "answer": "A long typed answer", "chosen": false}
            ]
        }]),
    );
    let tree = describe_settled(cx, &staged, "mods/composer", values, None).unwrap();
    for shown in [
        "composer-item-answer-tu1",
        "Rain or sun?",
        "You chose: Rain",
        "Anything else?",
        "You answered: A long typed answer",
    ] {
        assert!(tree.contains(shown), "missing {shown}: {tree}");
    }
}

fn needs_lead_values(needs_you: bool, agents: Value, terminals: Value) -> BTreeMap<String, Value> {
    let mut values = fixture(false, props(), updates("current", "current"));
    values.insert("vaultJson".into(), vault_notes(vec![]));
    values.insert(
        "lead".into(),
        json!({
            "needs_you": needs_you,
            "ask": null,
            "started": true,
            "state": "waiting",
            "terminal": "hidden",
            "conversation": {"version": 1, "length": 0}
        }),
    );
    values.insert("agents".into(), agents);
    values.insert("terminals".into(), terminals);
    values
}

fn lead_terminal() -> Value {
    json!([{"address": "t1", "state": "live", "labels": {"lead": "claude"}, "status": "blocked"}])
}

fn blocked_agent(address: &str) -> Value {
    json!([{"address": address, "pane": "p1", "status": "blocked", "title": "Build"}])
}

/// The bell shows the lead's row from needs_you, and never a second row for the lead's own terminal.
#[gpui::test]
fn the_bell_counts_the_lead_once(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe_settled(
        cx,
        &staged,
        "mods/bell",
        needs_lead_values(true, blocked_agent("t1"), lead_terminal()),
        None,
    )
    .unwrap();
    assert!(tree.contains("bell-lead"), "{tree}");
    assert!(
        !tree.contains("bell-agent-t1"),
        "the lead's own terminal is not a second row: {tree}"
    );
    let calm = describe_settled(
        cx,
        &staged,
        "mods/bell",
        needs_lead_values(false, blocked_agent("t1"), lead_terminal()),
        None,
    )
    .unwrap();
    assert!(!calm.contains("bell-lead"), "{calm}");
    assert!(!calm.contains("bell-agent-t1"), "{calm}");
}

fn topbar_needs_tree(cx: &mut TestAppContext, needs_you: bool, agents: Value) -> String {
    let staged = Staged::new();
    describe_settled(
        cx,
        &staged,
        "mods/topbar",
        needs_lead_values(needs_you, agents, lead_terminal()),
        None,
    )
    .unwrap()
}

/// The top bar's count has the lead's 1 only when needs_you says so, and still counts other blocked agents.
#[gpui::test]
fn the_top_bar_counts_the_lead_once(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let tree = topbar_needs_tree(cx, true, blocked_agent("t1"));
    assert!(tree.contains("1 needs you"), "{tree}");
    assert!(tree.contains("topbar-bell-count"), "{tree}");

    let quiet = topbar_needs_tree(cx, false, blocked_agent("t1"));
    assert!(!quiet.contains("1 needs you"), "the lead's blocked terminal alone counts nothing: {quiet}");
    assert!(!quiet.contains("topbar-bell-count"), "{quiet}");

    let other = topbar_needs_tree(cx, false, blocked_agent("t2"));
    assert!(other.contains("1 needs you"), "another agent still counts: {other}");
    assert!(other.contains("topbar-bell-count"), "{other}");
}
