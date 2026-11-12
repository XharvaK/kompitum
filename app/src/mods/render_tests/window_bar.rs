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

/// Until an agent joins through Connect, the status bar's right end holds nothing.
#[gpui::test]
fn the_status_bar_is_empty_until_an_agent_joins(cx: &mut TestAppContext) {
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
    assert!(tree.contains("lead-root"), "{tree}");
    for gone in ["lead-notes", "lead-door", "Notes", "Manager"] {
        assert!(!tree.contains(gone), "{gone} before anyone joined: {tree}");
    }
}

/// Once an agent joined, Notes sits at the status bar's right end; the joined system's app
/// is offered only when an app answers its link.
#[gpui::test]
fn the_status_bar_offers_notes_and_the_joined_systems_app(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let joined = || {
        let mut values = fixture(true, props(), updates("current", "current"));
        values.get_mut("settings").unwrap()["connect"] = json!({
            "grokbot": { "since": "2026-11-01T00:00:00Z", "agent": "manager", "name": "Grok Bot", "link": "grokbot://" }
        });
        values
    };
    let mut values = joined();
    values.insert("appLink".into(), json!({ "name": "Grok Bot", "icon": null }));
    let tree = describe_with_values(cx, &staged, "mods/lead", values, None).unwrap();
    for shown in ["lead-notes", "Notes with Manager", "lead-door", "Open Grok Bot", "Grok Bot"] {
        assert!(tree.contains(shown), "missing {shown}: {tree}");
    }
    assert!(!tree.contains("The Line"), "{tree}");
    assert!(!tree.contains("Claude"), "{tree}");

    // No app answers the link (the fixture's default): Notes stays, the app's item goes.
    let tree = describe_with_values(cx, &staged, "mods/lead", joined(), None).unwrap();
    assert!(tree.contains("lead-notes"), "{tree}");
    assert!(!tree.contains("lead-door"), "{tree}");
}

/// The top bar names the shown tab's folder: its focused pane's, with the home folder as "~".
#[gpui::test]
fn the_top_bar_shows_the_shown_tabs_folder(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe_with_values(
        cx,
        &staged,
        "mods/topbar",
        fixture(true, props(), updates("current", "current")),
        None,
    )
    .unwrap();
    assert!(tree.contains("topbar-folder"), "{tree}");
    assert!(tree.contains("~/project"), "{tree}");

    // A Windows folder under the home folder is shortened the same way, and one that only
    // starts with the same letters is not.
    let mut windows = fixture(true, props(), updates("current", "current"));
    windows.insert("home".into(), json!("C:\\Users\\me"));
    windows.get_mut("panes").unwrap()[0]["cwd"] = json!("C:\\Users\\me\\work");
    let tree = describe_with_values(cx, &staged, "mods/topbar", windows.clone(), None).unwrap();
    assert!(tree.contains("~\\work") || tree.contains("~\\\\work"), "{tree}");
    windows.get_mut("panes").unwrap()[0]["cwd"] = json!("C:\\Users\\merlin");
    let tree = describe_with_values(cx, &staged, "mods/topbar", windows, None).unwrap();
    assert!(!tree.contains("~rlin"), "{tree}");
}

/// A terminal tab that is no agent and not the lead gets a row under "Terminals".
#[gpui::test]
fn the_sidebar_lists_a_terminal_that_is_no_agent(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut values = fixture(true, props(), updates("current", "current"));
    values.get_mut("spaces").unwrap()[0]["tabs"].as_array_mut().unwrap().push(json!({
        "id": "tab3", "index": 2, "name": "Scratch", "active": false, "status": "idle", "panes": 1,
        "zoomed": false, "settled": false, "settled_at": null, "changed_at": null, "working_since": null
    }));
    values.get_mut("panes").unwrap().as_array_mut().unwrap().push(json!({
        "pane": "p3", "terminal": "t3", "tab": "tab3", "title": "Scratch", "cwd": "/fixture", "branch": null,
        "status": "idle", "app": null, "program": {"key": "shell", "name": "Shell", "icon": null, "tint": true, "shell": true},
        "focused": false, "zoomed": false
    }));
    let tree = describe_with_values(cx, &staged, "mods/agents", values, None).unwrap();
    for id in ["ag-terminals", "Terminals", "term-tab3"] {
        assert!(tree.contains(id), "missing {id}: {tree}");
    }
}

/// Every terminal tab is an agent's: the sidebar has no "Terminals" section.
#[gpui::test]
fn the_sidebar_has_no_terminals_section_without_one(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe_with_values(
        cx,
        &staged,
        "mods/agents",
        fixture(true, props(), updates("current", "current")),
        None,
    )
    .unwrap();
    assert!(!tree.contains("ag-terminals"), "{tree}");
    assert!(!tree.contains("Terminals"), "{tree}");
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

