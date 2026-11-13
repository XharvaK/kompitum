//! Render tests for the window's bars (audit/REWORK-2026-11-09.md, W1 to W4): see the parent module for the
//! helpers.
#![allow(unused_imports)]

use super::*;

/// The top bar is a breadcrumb of where you are, and it still holds the Files panel button; the bell and
/// the mailbox moved to the sidebar's foot.
#[gpui::test]
fn the_top_bar_is_a_breadcrumb_and_the_panel_toggles(cx: &mut TestAppContext) {
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
    for id in ["topbar-crumbs", "panel-files"] {
        assert!(tree.contains(id), "missing {id}: {tree}");
    }
    for gone in ["topbar-bell", "topbar-mail", "topbar-back"] {
        assert!(!tree.contains(gone), "{gone} moved out of the top bar: {tree}");
    }
}

/// The brand row holds the space's "+" beside the sidebar toggle; the
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

/// The brand row no longer holds a search button: Search is the first row of the sidebar's nav.
#[gpui::test]
fn the_brand_row_has_no_search_button(cx: &mut TestAppContext) {
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
    assert!(tree.contains("space-menu"), "the brand row still has the space menu: {tree}");
    assert!(!tree.contains("\"search\""), "the brand row has no search button: {tree}");

    // The control: the same id, quoted, is what the nav's Search row shows.
    let nav = describe_with_values(
        cx,
        &staged,
        "mods/workspace-chrome",
        fixture(true, json!({"nav": true}), updates("current", "current")),
        None,
    )
    .unwrap();
    assert!(nav.contains("\"sidebar-search\""), "ids show quoted: {nav}");
}

/// The sidebar's nav offers Search and Tasks; with no tasks it says so and shows no counts.
#[gpui::test]
fn the_sidebar_offers_search_and_tasks(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut values = fixture(true, json!({"nav": true}), updates("current", "current"));
    values.insert(
        "statusJson".into(),
        json!({"tasks": {"review": [], "running": [], "queued": []}}),
    );
    let tree = describe_settled(cx, &staged, "mods/workspace-chrome", values, None).unwrap();
    for shown in ["sidebar-search", "Search", "sidebar-tasks", "Tasks", "No tasks"] {
        assert!(tree.contains(shown), "missing {shown}: {tree}");
    }
    for gone in ["sidebar-tasks-review", "sidebar-tasks-count"] {
        assert!(!tree.contains(gone), "{gone} with no tasks: {tree}");
    }
}

/// The sidebar's foot holds Settings, Limits and the bell; the notes moved into the bell, so no Notes button is here.
#[gpui::test]
fn the_foot_holds_settings_limits_and_the_bell(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut values = fixture(true, json!({"footer": true}), updates("current", "current"));
    values.insert("vaultJson".into(), vault_notes(vec![]));
    let tree = describe_settled(cx, &staged, "mods/workspace-chrome", values, None).unwrap();
    for shown in ["settings", "sidebar-usage", "Limits", "sidebar-bell", "Nothing needs you"] {
        assert!(tree.contains(shown), "missing {shown}: {tree}");
    }
    assert!(!tree.contains("sidebar-notes"), "the notes moved into the bell: {tree}");
}

/// Limits' tooltip names Claude's windows that have a percentage, so the Owner sees the higher one at a glance.
#[gpui::test]
fn limits_names_claudes_windows_in_its_tooltip(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut values = fixture(true, json!({"footer": true}), updates("current", "current"));
    values.insert("vaultJson".into(), vault_notes(vec![]));
    values.insert(
        "lead".into(),
        json!({"state": "waiting", "ask": null, "usage": {
            "context_tokens": 300000, "compact_at": 444000,
            "five_hour": {"used_percentage": 82, "resets_at": null}, "seven_day": null
        }}),
    );
    let tree = describe_settled(cx, &staged, "mods/workspace-chrome", values, None).unwrap();
    assert!(tree.contains("sidebar-usage"), "{tree}");
    assert!(tree.contains("5h 82%"), "{tree}");
}

/// The bell at the sidebar's foot counts what needs you (blocked agents, the lead's ask, ready updates) and
/// the notes waiting. Tasks waiting for review don't count; it stays away at zero and caps at 99+.
#[gpui::test]
fn the_bell_at_the_foot_counts_what_needs_you(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();

    let mut values = fixture(true, json!({"footer": true}), updates("current", "current"));
    values.insert("vaultJson".into(), vault_notes(vec![]));
    values.insert(
        "statusJson".into(),
        json!({"tasks": {"review": [{"name": "fix-docs"}], "running": [], "queued": []}, "waiting": {"review": 120}}),
    );
    let tree = describe_settled(cx, &staged, "mods/workspace-chrome", values, None).unwrap();
    assert!(!tree.contains("sidebar-bell-count"), "tasks for review don't count: {tree}");

    let blocked: Vec<Value> = (0..100)
        .map(|i| json!({"address": format!("b{i}"), "pane": "p1", "status": "blocked", "title": "Stuck"}))
        .collect();
    let mut values = fixture(true, json!({"footer": true}), updates("current", "current"));
    values.insert("vaultJson".into(), vault_notes(vec![]));
    values.insert("agents".into(), Value::Array(blocked));
    let tree = describe_settled(cx, &staged, "mods/workspace-chrome", values, None).unwrap();
    assert!(tree.contains("99+"), "the count caps at 99+: {tree}");

    let mut values = fixture(true, json!({"footer": true}), updates("current", "current"));
    values.insert("vaultJson".into(), vault_notes(vec![]));
    values.insert(
        "agents".into(),
        json!([{"address": "t1", "pane": "p1", "status": "blocked", "title": "Build"}]),
    );
    values.insert(
        "lead".into(),
        json!({"state": "asking", "ask": {"id": "a1", "terminal": "t1", "kind": "permission", "input": {}}}),
    );
    let tree = describe_settled(cx, &staged, "mods/workspace-chrome", values, None).unwrap();
    assert!(tree.contains("sidebar-bell-count"), "blocked + ask: {tree}");
    assert!(tree.contains(r#"text "2""#), "one blocked agent and the ask make 2: {tree}");
    assert!(tree.contains("2 need you"), "{tree}");

    let mut values = fixture(true, json!({"footer": true}), updates("current", "current"));
    values.insert(
        "vaultJson".into(),
        vault_notes(vec![vault_owner_note(), vault_agent_note()]),
    );
    let tree = describe_settled(cx, &staged, "mods/workspace-chrome", values, None).unwrap();
    assert!(tree.contains(r#"text "2""#), "two notes make 2: {tree}");
    assert!(tree.contains("2 notes"), "{tree}");

    let mut values = fixture(true, json!({"footer": true}), updates("current", "current"));
    values.insert("vaultJson".into(), vault_notes(vec![vault_owner_note()]));
    values.insert(
        "agents".into(),
        json!([{"address": "t1", "pane": "p1", "status": "blocked", "title": "Build"}]),
    );
    let tree = describe_settled(cx, &staged, "mods/workspace-chrome", values, None).unwrap();
    assert!(tree.contains("1 needs you · 1 note"), "{tree}");

    let mut values = fixture(true, json!({"footer": true}), updates("current", "current"));
    values.insert("vaultJson".into(), vault_notes(vec![]));
    let tree = describe_settled(cx, &staged, "mods/workspace-chrome", values, None).unwrap();
    assert!(!tree.contains("sidebar-bell-count"), "nothing waits: {tree}");
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

/// Once an agent joined, Notes sits at the status bar's right end; the joined system's app is offered only
/// when an app answers its link.
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

/// The Notes window links to the joined system's app in its header, when an app answers the link.
#[gpui::test]
fn the_notes_window_links_to_the_joined_systems_app(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let window_props = json!({ "with": "manager", "name": "Grok Bot", "link": "grokbot://" });
    let mut values = fixture(true, window_props.clone(), updates("current", "current"));
    values.insert("appLink".into(), json!({ "name": "Grok Bot", "icon": null }));
    let tree = describe_with_values(cx, &staged, "mods/line", values, None).unwrap();
    for shown in ["line-door", "Open Grok Bot"] {
        assert!(tree.contains(shown), "missing {shown}: {tree}");
    }

    let tree = describe_with_values(
        cx,
        &staged,
        "mods/line",
        fixture(true, window_props, updates("current", "current")),
        None,
    )
    .unwrap();
    assert!(!tree.contains("line-door"), "no app answers the link: {tree}");
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

/// The breadcrumb names the shown tab's branch and the space's machine, each with its icon.
#[gpui::test]
fn the_top_bar_names_the_branch_and_the_machine(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut values = fixture(true, props(), updates("current", "current"));
    values.get_mut("spaces").unwrap()[0]["machine"] = json!("laptop");
    let tree = describe_with_values(cx, &staged, "mods/topbar", values, None).unwrap();
    for shown in ["topbar-branch", "main", "topbar-machine", "laptop"] {
        assert!(tree.contains(shown), "missing {shown}: {tree}");
    }
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

/// A failed `kom status --json` keeps the last counts, and the Tasks row's tooltip must name the failure's
/// first line.
#[gpui::test]
fn a_failed_status_read_names_the_error_on_tasks(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut values = fixture(true, json!({"nav": true}), updates("current", "current"));
    values.insert(
        "statusFail".into(),
        json!("could not lock the vault\nand a stack dump"),
    );
    let tree = describe_settled(
        cx,
        &staged,
        "mods/workspace-chrome",
        values,
        Some("this.review = 5;"),
    )
    .unwrap();
    assert!(
        tree.contains("sidebar-tasks-review"),
        "the last review count is kept: {tree}"
    );
    assert!(
        tree.contains(r#"text "5""#),
        "the last review count (5) is still shown: {tree}"
    );
    assert!(
        tree.contains("Tasks · couldn't read them: could not lock the vault"),
        "{tree}"
    );
    assert!(
        !tree.contains("and a stack dump"),
        "only the first line of the error: {tree}"
    );
}

/// A status read that works again counts the tasks: 2 running and 1 queued make 3 running, and 1 to review
/// makes the review pill; the error goes.
#[gpui::test]
fn a_working_status_read_counts_the_tasks(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut values = fixture(true, json!({"nav": true}), updates("current", "current"));
    values.insert(
        "statusJson".into(),
        json!({"tasks": {
            "review": [{"name": "fix-docs"}],
            "running": [{"name": "a"}, {"name": "b"}],
            "queued": [{"name": "c"}]
        }}),
    );
    let tree = describe_settled(cx, &staged, "mods/workspace-chrome", values, None).unwrap();
    assert!(tree.contains("sidebar-tasks-review"), "{tree}");
    assert!(tree.contains(r#"text "1""#), "{tree}");
    assert!(tree.contains("3 running · 1 to review"), "{tree}");
    assert!(
        !tree.contains("couldn't read them"),
        "a good read has no error: {tree}"
    );
}
