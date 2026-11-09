//! Render tests for the window's bars (audit/REWORK-2026-11-09.md, W1 to W4): see the parent module for the
//! helpers.
#![allow(unused_imports)]

use super::*;

/// The top bar carries the Board's bell, the mailbox and the Files panel button.
#[gpui::test]
fn the_top_bar_shows_the_bell_the_mailbox_and_files(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe(
        cx,
        &staged,
        "mods/topbar",
        true,
        props(),
        updates("current", "current"),
        None,
    )
    .unwrap();
    for id in ["topbar-bell", "topbar-mail", "panel-files"] {
        assert!(tree.contains(id), "missing {id}: {tree}");
    }
}

/// Back belongs to a live lead that is not the tab on screen.
#[gpui::test]
fn the_top_bar_shows_back_only_when_the_lead_is_elsewhere(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut elsewhere = fixture(true, props(), updates("current", "current"));
    elsewhere.get_mut("terminals").unwrap()[1]["labels"] = json!({ "lead": "claude" });
    let tree = describe_with_values(cx, &staged, "mods/topbar", elsewhere, None).unwrap();
    assert!(tree.contains("topbar-back"), "the lead is in another tab: {tree}");
    assert!(tree.contains("Back to the lead"), "{tree}");

    let mut here = fixture(true, props(), updates("current", "current"));
    here.get_mut("terminals").unwrap()[0]["labels"] = json!({ "lead": "claude" });
    let tree = describe_with_values(cx, &staged, "mods/topbar", here, None).unwrap();
    assert!(
        !tree.contains("topbar-back"),
        "the lead's tab is the active one: {tree}"
    );
}

/// The brand row holds the space's "+" beside search and the sidebar toggle; the
/// menu shows the current space, a new terminal and the installed agents only.
#[gpui::test]
fn the_brand_row_shows_the_space_menu(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe_with_values(
        cx,
        &staged,
        "mods/workspace-chrome",
        fixture(true, props(), updates("current", "current")),
        None,
    )
    .unwrap();
    assert!(tree.contains("space-menu"), "{tree}");
    assert!(tree.contains("Space and new terminals"), "{tree}");

    let setup = "this.spaceOpen = true; this.agents = [\
        {key:'claude', name:'Claude Code', icon:'icons/claude.svg', tint:false, installed:true}, \
        {key:'codex', name:'Codex', icon:null, tint:true, installed:false}];";
    let tree = describe_with_values(
        cx,
        &staged,
        "mods/workspace-chrome",
        fixture(true, props(), updates("current", "current")),
        Some(setup),
    )
    .unwrap();
    for id in ["space-current", "space-new-terminal", "space-new-claude"] {
        assert!(tree.contains(id), "missing {id}: {tree}");
    }
    assert!(
        !tree.contains("space-new-codex"),
        "only installed agents get a row: {tree}"
    );
}

/// The bell's badge counts what waits: blocked agents, the lead's ask and the
/// tasks waiting for review; it stays away at zero and caps at 99+.
#[gpui::test]
fn the_top_bar_bell_badge_counts_what_waits(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();

    let mut values = fixture(true, props(), updates("current", "current"));
    values.insert(
        "statusJson".into(),
        json!({"tasks": {"review": [], "running": [], "queued": []}, "waiting": {"review": 120}}),
    );
    let tree = describe_settled(cx, &staged, "mods/topbar", values, None).unwrap();
    assert!(tree.contains("topbar-bell-count"), "120 waiting: {tree}");
    assert!(tree.contains("99+"), "the count caps at 99+: {tree}");

    let mut values = fixture(true, props(), updates("current", "current"));
    values.get_mut("agents").unwrap()[0]["status"] = json!("blocked");
    values.insert(
        "lead".into(),
        json!({"state": "asking", "ask": {"id": "a1", "terminal": "t1", "kind": "permission", "input": {}}}),
    );
    values.insert(
        "statusJson".into(),
        json!({"tasks": {"review": [], "running": [], "queued": []}, "waiting": {"review": 2}}),
    );
    let tree = describe_settled(cx, &staged, "mods/topbar", values, None).unwrap();
    assert!(tree.contains("topbar-bell-count"), "blocked + ask + review: {tree}");

    let tree = describe_settled(
        cx,
        &staged,
        "mods/topbar",
        fixture(true, props(), updates("current", "current")),
        None,
    )
    .unwrap();
    assert!(!tree.contains("topbar-bell-count"), "nothing waits: {tree}");
}

/// The lead's status-bar pill carries the Line right beside it.
#[gpui::test]
fn the_lead_pill_shows_the_line(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe(
        cx,
        &staged,
        "mods/lead",
        true,
        props(),
        updates("current", "current"),
        None,
    )
    .unwrap();
    assert!(tree.contains("lead-line"), "{tree}");
    assert!(tree.contains("The Line"), "{tree}");
}

/// A failed `kom status --json` must not wipe the last review count, and the
/// bell's tooltip must name the failure's first line.
#[gpui::test]
fn a_failed_status_read_keeps_the_bells_other_counts(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut values = fixture(true, props(), updates("current", "current"));
    values.get_mut("agents").unwrap()[0]["status"] = json!("blocked");
    values.insert(
        "lead".into(),
        json!({"state": "asking", "ask": {"id": "a1", "terminal": "t1", "kind": "permission", "input": {}}}),
    );
    values.insert(
        "statusFail".into(),
        json!("could not lock the vault\nand a stack dump"),
    );
    let tree = describe_settled(
        cx,
        &staged,
        "mods/topbar",
        values,
        Some("this.review = 5;"),
    )
    .unwrap();
    assert!(
        tree.contains("topbar-bell-count"),
        "the last review count is kept with the other waits: {tree}"
    );
    assert!(
        tree.contains(r#"text "7""#),
        "blocked + ask + last review (5) is 7: {tree}"
    );
    assert!(
        tree.contains("Board · couldn't read the tasks waiting for review: could not lock the vault"),
        "{tree}"
    );
    assert!(
        !tree.contains("and a stack dump"),
        "only the first line of the error: {tree}"
    );
}

/// A status read that works again drops the error from the tooltip.
#[gpui::test]
fn a_working_status_read_clears_the_bells_error(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut values = fixture(true, props(), updates("current", "current"));
    values.insert(
        "statusJson".into(),
        json!({"tasks": {"review": [], "running": [], "queued": []}, "waiting": {"review": 3}}),
    );
    let tree = describe_settled(
        cx,
        &staged,
        "mods/topbar",
        values,
        Some("this.reviewError = 'stale dump';"),
    )
    .unwrap();
    assert!(tree.contains("topbar-bell-count"), "{tree}");
    assert!(tree.contains(r#"text "3""#), "{tree}");
    assert!(
        !tree.contains("couldn't read the tasks waiting for review"),
        "a good read clears the error: {tree}"
    );
}

