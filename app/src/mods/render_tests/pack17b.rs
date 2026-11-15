//! Pack 17b: the context ring, the three Limits cards, the lead's own row and its menu.
#![allow(unused_imports)]

use super::*;

use gpui::{Modifiers, MouseButton};

/// A lead waiting with the given context count and two Claude windows.
fn lead_with_usage(tokens: u64, five_hour: Value, seven_day: Value) -> Value {
    json!({
        "state": "waiting",
        "ask": null,
        "started": true,
        "terminal": "hidden",
        "reveal": null,
        "reveal_why": null,
        "conversation": { "version": 1, "length": 0 },
        "model": null,
        "usage": {
            "context_tokens": tokens,
            "context_window": 1000,
            "compact_at": 1000,
            "cache_ttl": null,
            "cache_expires_at": null,
            "five_hour": five_hour,
            "seven_day": seven_day,
            "session_id": null,
            "seen_ms": 0
        },
        "compacting": false
    })
}

fn topbar_draws(cx: &mut TestAppContext, lead: Value) -> String {
    let staged = Staged::new();
    let mut values = fixture(true, props(), updates("current", "current"));
    values.insert("vaultJson".into(), vault_notes(vec![]));
    values.insert("lead".into(), lead);
    describe_settled(cx, &staged, "mods/topbar", values, None).unwrap()
}

/// The ring follows the context: calm under 75%, the warning colour from 75%, and the alarm colour from 95%.
#[gpui::test]
fn the_top_bars_ring_shows_the_contexts_colour(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    for (tokens, tone) in [(300, "done"), (800, "working"), (960, "blocked")] {
        let tree = topbar_draws(cx, lead_with_usage(tokens, Value::Null, Value::Null));
        assert!(
            tree.contains(&format!("topbar-context-{tone}")),
            "at {tokens} tokens the ring is {tone}: {tree}"
        );
        assert!(!tree.contains("gauge.svg"), "a known context draws the ring, not the gauge: {tree}");
    }
}

/// A Claude window at 75% or more puts the dot at the ring's top right.
#[gpui::test]
fn the_top_bars_dot_marks_a_plan_window_at_three_quarters(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let tree = topbar_draws(
        cx,
        lead_with_usage(300, Value::Null, json!({"used_percentage": 80, "resets_at": null})),
    );
    assert!(tree.contains("topbar-limits-dot"), "a week at 80% shows the dot: {tree}");
    let bare = topbar_draws(
        cx,
        lead_with_usage(300, Value::Null, json!({"used_percentage": 60, "resets_at": null})),
    );
    assert!(!bare.contains("topbar-limits-dot"), "a week at 60% shows no dot: {bare}");
}

/// With no lead the gauge stays, and the tooltip keeps today's words.
#[gpui::test]
fn the_top_bar_keeps_the_gauge_without_a_lead(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let tree = topbar_draws(cx, Value::Null);
    assert!(tree.contains("gauge.svg"), "no lead leaves the gauge: {tree}");
    assert!(tree.contains("topbar-limits"), "{tree}");
    assert!(!tree.contains("topbar-context-"), "no ring without a context: {tree}");
}

fn no_workers() -> Value {
    json!({"workers": {"chain": [], "next": null, "workers": []}})
}

fn limits_draws(cx: &mut TestAppContext, usage: Value, status: Value, props: Value) -> String {
    let staged = Staged::new();
    let mut values = fixture(false, props, updates("current", "current"));
    values.insert("statusJson".into(), status);
    values.insert("lead".into(), lead_with_usage(0, Value::Null, Value::Null));
    if let Some(lead) = values.get_mut("lead") {
        lead["usage"] = usage;
    }
    describe_settled(cx, &staged, "mods/limits", values, None).unwrap()
}

/// The popup is three plain cards: this conversation, Claude's plan and the workers.
#[gpui::test]
fn the_limits_popup_draws_three_cards(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let usage = json!({
        "context_tokens": 271000,
        "context_window": 1000000,
        "compact_at": 444000,
        "five_hour": {"used_percentage": 41, "resets_at": null},
        "seven_day": {"used_percentage": 73, "resets_at": null}
    });
    let tree = limits_draws(
        cx,
        usage,
        no_workers(),
        json!({"overlay": "o1", "terminal": "t1"}),
    );
    for shown in [
        "This conversation",
        "Claude plan",
        "Workers",
        "271K of 444K",
        "41%",
        "73%",
    ] {
        assert!(tree.contains(shown), "missing {shown}: {tree}");
    }
    assert!(!tree.contains("text \"Context\""), "the old Context heading is gone: {tree}");
    let card = tree
        .lines()
        .find(|line| line.contains(":id[Str(\"limits-conversation\")]"))
        .unwrap_or_else(|| panic!("the conversation card is drawn: {tree}"));
    assert!(card.contains(".rounded[Number(10.0)]"), "{card}");
    assert!(card.contains(".p[Number(12.0)]"), "{card}");
}

/// A used-up window fills its dial with the alarm colour and says "Used up", with when it is back under its name.
#[gpui::test]
fn a_used_up_claude_window_reads_used_up(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let usage = json!({
        "context_tokens": 100000,
        "context_window": 1000000,
        "compact_at": 444000,
        "five_hour": {"used_percentage": 100, "resets_at": now + 2 * 3600},
        "seven_day": {"used_percentage": 20, "resets_at": null}
    });
    let tree = limits_draws(cx, usage, no_workers(), json!({"overlay": "o1"}));
    assert!(tree.contains("text \"Used up\""), "{tree}");
    assert!(tree.contains("text \"back "), "the name says when it is back: {tree}");
}

/// Compact now is centred and only as wide as its words, and the tidy-up line is gone.
#[gpui::test]
fn compact_now_is_centred_and_the_tidy_line_is_gone(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let usage = json!({
        "context_tokens": 100000,
        "context_window": 1000000,
        "compact_at": 444000,
        "five_hour": null,
        "seven_day": null
    });
    let tree = limits_draws(
        cx,
        usage,
        no_workers(),
        json!({"overlay": "o1", "terminal": "t1"}),
    );
    let row = tree
        .lines()
        .find(|line| line.contains(":id[Str(\"limits-compact-row\")]"))
        .unwrap_or_else(|| panic!("Compact now has a centring row: {tree}"));
    assert!(row.contains(".justify_center"), "{row}");
    let button = tree
        .lines()
        .find(|line| line.contains(":id[Str(\"limits-compact\")]"))
        .unwrap_or_else(|| panic!("Compact now is drawn: {tree}"));
    assert!(!button.contains(".w_full"), "the button is only as wide as its words: {button}");
    assert!(button.contains(".px[Number(16.0)]"), "{button}");
    assert!(
        !tree.contains("Kompitum tidies up by itself"),
        "the tidy-up line is gone: {tree}"
    );
}

/// Opened from the top bar, with no terminal of its own, Compact now runs in the live lead's terminal.
#[gpui::test]
fn compact_now_finds_the_leads_terminal_from_the_top_bar(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut values = fixture(false, json!({"overlay": "o1"}), updates("current", "current"));
    values.insert("statusJson".into(), no_workers());
    values.insert("lead".into(), lead_with_usage(500, Value::Null, Value::Null));
    values.insert(
        "terminals".into(),
        json!([{"address": "t9", "name": "Claude", "title": "Claude", "argv": ["claude"], "parent": null,
            "labels": {"lead": "claude"}, "state": "live", "status": null, "report": {}, "program": null,
            "cwd": "/fixture/project"}]),
    );
    let tree = describe_settled(cx, &staged, "mods/limits", values, None).unwrap();
    assert!(tree.contains("limits-compact"), "{tree}");
}

/// A lead that reports a status stays in its own row above every group, and no group counts it.
#[gpui::test]
fn the_lead_row_sits_above_the_groups_and_is_not_counted(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut values = with_lead_terminal();
    let program = values["agents"][0]["program"].clone();
    values.get_mut("agents").unwrap().as_array_mut().unwrap().push(json!({
        "address": "t9", "pane": "p1", "space": "s1", "space_name": "Local",
        "tab": "tab1", "tab_index": 1, "tab_name": "Build", "status": "idle",
        "app": "claude", "program": program, "title": "Lead", "seq": 9, "focused": false, "pinned": false
    }));
    let tree = describe_with_values(cx, &staged, "mods/agents", values, None).unwrap();
    let lead = tree.find("ag-lead").unwrap_or_else(|| panic!("the lead row is drawn: {tree}"));
    let working = tree.find("Working").unwrap_or_else(|| panic!("the working group is drawn: {tree}"));
    let agent = tree.find("ag-t1").unwrap();
    assert!(lead < working && working < agent, "the lead is above every group: {tree}");
    assert!(!tree.contains("text \"Idle\""), "no group counts the lead: {tree}");
}

fn mount_mod(
    cx: &mut TestAppContext,
    staged: &Staged,
    entry: &str,
    values: BTreeMap<String, Value>,
) -> (gpui::WindowHandle<Mounted>, VisualTestContext, Rc<RefCell<Calls>>) {
    let calls = Rc::new(RefCell::new(Calls::default()));
    let policy = Policy::new()
        .with_storage_path(staged.0.join("store-pack17b.json"))
        .with_capabilities(Capabilities::new().storage(true).read_roots([staged.0.clone()]))
        .with_host_module(module(values, calls.clone()))
        .unwrap();
    gpui_shell::policy::set_default(policy);
    let runtime = ShellRuntime::new_isolated().unwrap();
    let window = cx.add_window(|window, cx| Mounted(runtime.try_load(&staged.0.join(entry), window, cx).unwrap()));
    let mut visual = VisualTestContext::from_window(*window.deref(), cx);
    visual.run_until_parked();
    draw(&mut visual);
    (window, visual, calls)
}

fn draw(visual: &mut VisualTestContext) {
    visual.update(|window, cx| {
        window.refresh();
        window.draw(cx).clear(cx)
    });
    visual.run_until_parked();
}

/// A right click on the lead's row opens the menu's three lead items.
#[gpui::test]
fn a_right_click_on_the_lead_opens_its_menu(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let (window, mut visual, calls) = mount_mod(cx, &staged, "mods/agents", with_lead_terminal());
    let at = visual.debug_bounds("ag-lead").expect("the lead row is laid out").center();
    visual.simulate_mouse_move(at, None, Modifiers::default());
    visual.simulate_mouse_down(at, MouseButton::Right, Modifiers::default());
    visual.simulate_mouse_up(at, MouseButton::Right, Modifiers::default());
    visual.run_until_parked();
    draw(&mut visual);
    let spec = calls
        .borrow()
        .effects
        .iter()
        .find(|(name, _)| name == "openOverlay")
        .and_then(|(_, args)| args.first().cloned())
        .unwrap_or_else(|| panic!("the lead's menu opens: {:?}", calls.borrow().effects));
    assert_eq!(spec["mod"], "menu", "{spec}");
    assert_eq!(spec["props"]["tab"], "lead", "{spec}");
    assert_eq!(spec["props"]["target"]["terminal"], "t9", "{spec}");
    drop(visual);
    cx.run_until_parked();
    window.update(cx, |_, window, _| window.remove_window()).ok();
    cx.run_until_parked();

    let menu_props = spec["props"].clone();
    let tree = describe_settled(
        cx,
        &staged,
        "mods/menu",
        fixture(false, menu_props, updates("current", "current")),
        None,
    )
    .unwrap();
    for shown in ["Show terminal", "Restart lead", "Copy folder path"] {
        assert!(tree.contains(shown), "missing {shown}: {tree}");
    }
}

/// Text outside the composer and Settings is one step larger: the top bar's folder was 13, now 14.
#[gpui::test]
fn interface_text_moves_one_step_larger(cx: &mut TestAppContext) {
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
    let line = tree
        .lines()
        .find(|line| line.contains(":id[Str(\"topbar-folder\")]"))
        .unwrap_or_else(|| panic!("the top bar's folder is drawn: {tree}"));
    assert!(line.contains(".text_size[Number(14.0)]"), "{line}");
}
