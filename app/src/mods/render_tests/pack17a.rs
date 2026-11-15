//! Render tests for pack 17a: the permission mode in Settings, the mode-change toast, the quieter
//! composer and the jump-to-latest pill.
#![allow(unused_imports)]

use super::*;
use gpui::{Entity, IntoElement, ParentElement as _, Render, Styled as _, VisualTestContext};
use std::cell::Cell;
use std::ops::Deref;

fn permission_values(mode: Option<&str>) -> BTreeMap<String, Value> {
    let mut p = props();
    p["section"] = json!("agents");
    let mut values = fixture(true, p, updates("current", "current"));
    values.insert("knownAgents".into(), agent_catalog());
    values.insert(
        "workersJson".into(),
        agent_workers(json!([]), json!([])),
    );
    values.insert(
        "statusJson".into(),
        json!({"tasks": {"review": [], "running": [], "queued": []}, "claude": null}),
    );
    let mut lead = json!({"program": "claude"});
    if let Some(mode) = mode {
        lead["permission_mode"] = json!(mode);
    }
    values.insert("person".into(), json!({"lead": lead}));
    values
}

/// Settings › Agents lists the four permission modes, each with its line, and marks Auto when the
/// setting is absent.
#[gpui::test]
fn the_permission_mode_row_lists_four_modes_and_marks_auto(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe_settled(
        cx,
        &staged,
        "frame/settings",
        permission_values(None),
        Some("this.agentPage = { leadPermissionMode: { open: true }, errors: [], usage: {}, usageError: {}, usageAt: {}, listModels: {}, leadCli: {}, leadModel: {}, leadEffort: {}, known: null, workers: null, status: null, workerError: null, busy: false, addOpen: false, add: {}, drag: null };"),
    )
    .unwrap();
    assert!(tree.contains("st-lead-permission-mode"), "{tree}");
    for word in ["Ask first", "Accept edits", "Plan first", "Auto"] {
        assert!(tree.contains(word), "missing {word:?}: {tree}");
    }
    for line in [
        "Asks before it edits files or runs commands.",
        "Edits files without asking; asks before commands.",
        "Plans and changes nothing until you approve.",
        "Approves routine actions itself; asks about risky ones.",
    ] {
        assert!(tree.contains(line), "missing {line:?}: {tree}");
    }
    let lines: Vec<&str> = tree.lines().collect();
    let at = lines
        .iter()
        .position(|line| line.contains("st-lead-permission-mode-auto"))
        .unwrap_or_else(|| panic!("the Auto row is missing: {tree}"));
    let row = lines[at..(at + 8).min(lines.len())].join("\n");
    assert!(row.contains("check.svg"), "Auto is marked: {row}");
    let at = lines
        .iter()
        .position(|line| line.contains("st-lead-permission-mode-plan"))
        .unwrap_or_else(|| panic!("the Plan first row is missing: {tree}"));
    let row = lines[at..(at + 8).min(lines.len())].join("\n");
    assert!(!row.contains("check.svg"), "another mode is not marked: {row}");
}

fn composer_mode_values(mode: &str) -> BTreeMap<String, Value> {
    let mut values = fixture(true, props(), updates("current", "current"));
    values.insert("knownAgents".into(), agent_catalog());
    values.insert(
        "lead".into(),
        json!({
            "state": "waiting",
            "ask": null,
            "started": true,
            "terminal": "hidden",
            "conversation": { "version": 1, "length": 0 },
            "permission_mode": mode,
            "mode_guessed": false
        }),
    );
    values.insert("person".into(), json!({"lead": {"program": "claude"}}));
    values
}

/// Mounts the composer with `values`, runs `setup`, and returns the effects it recorded.
fn composer_effects(cx: &mut TestAppContext, values: BTreeMap<String, Value>, setup: &str) -> Vec<Vec<Value>> {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let calls = Rc::new(RefCell::new(Calls::default()));
    let policy = Policy::new()
        .with_storage_path(staged.0.join("store-pack17a.json"))
        .with_capabilities(Capabilities::new().storage(true).read_roots([staged.0.clone()]))
        .with_host_module(module(values, calls.clone()))
        .unwrap();
    gpui_shell::policy::set_default(policy);
    let runtime = ShellRuntime::new_isolated().unwrap();
    let root = staged.wrapper("mods/composer", setup);
    let window = cx.add_window(|window, cx| Mounted(runtime.try_load(&root, window, cx).unwrap()));
    let mut visual = VisualTestContext::from_window(*window.deref(), cx);
    for _ in 0..2 {
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        visual.run_until_parked();
    }
    drop(visual);
    cx.run_until_parked();
    window.update(cx, |_, window, _| window.remove_window()).ok();
    cx.run_until_parked();
    let calls = calls.borrow();
    assert!(calls.unexpected.is_empty(), "{:?}", calls.unexpected);
    calls.effects.iter().map(|(_, args)| args.clone()).collect()
}

/// A confirmed mode that changes posts one info toast; the first confirmed mode posts nothing.
#[gpui::test]
fn a_changed_permission_mode_posts_one_toast_and_the_first_is_quiet(cx: &mut TestAppContext) {
    let first = composer_effects(
        cx,
        composer_mode_values("default"),
        "this.trackMode('default', cx);",
    );
    assert!(
        !first
            .iter()
            .any(|args| args.first().is_some_and(|n| n["title"].as_str() == Some("Permission mode: Ask first"))),
        "the first report is quiet: {first:?}"
    );

    let changed = composer_effects(
        cx,
        composer_mode_values("auto"),
        "this.trackMode('default', cx); this.trackMode('auto', cx);",
    );
    let toasts: Vec<&Vec<Value>> = changed
        .iter()
        .filter(|args| args.first().is_some_and(|n| n["title"].as_str() == Some("Permission mode: Auto")))
        .collect();
    assert_eq!(toasts.len(), 1, "one toast for the change: {changed:?}");
    assert_eq!(toasts[0][0]["level"], json!("info"));
}

fn composer_conversation(count: usize) -> Value {
    Value::Array(
        (0..count)
            .map(|n| json!({"kind": "notice", "id": format!("n{n}"), "text": format!("Line {n}")}))
            .collect(),
    )
}

fn composer_values(items: Value) -> BTreeMap<String, Value> {
    let mut values = composer_mode_values("default");
    let length = items.as_array().map(Vec::len).unwrap_or(0);
    values.insert("conversation".into(), items);
    values.get_mut("lead").unwrap()["conversation"] = json!({ "version": 1, "length": length });
    values
}

struct Framed(Entity<ShellRoot>, Rc<Cell<f32>>);
impl Render for Framed {
    fn render(&mut self, _: &mut gpui::Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        gpui::div()
            .w_full()
            .h(gpui::px(self.1.get()))
            .relative()
            .child(gpui::div().absolute().inset_0().child(self.0.clone()))
    }
}

fn mount_composer(
    cx: &mut TestAppContext,
    values: BTreeMap<String, Value>,
    setup: &str,
    framed: Option<Rc<Cell<f32>>>,
) -> (VisualTestContext, gpui::WindowHandle<Framed>, Rc<RefCell<Calls>>) {
    let staged = Staged::new();
    let calls = Rc::new(RefCell::new(Calls::default()));
    let policy = Policy::new()
        .with_storage_path(staged.0.join("store-pack17a-pill.json"))
        .with_capabilities(Capabilities::new().storage(true).read_roots([staged.0.clone()]))
        .with_host_module(module(values, calls.clone()))
        .unwrap();
    gpui_shell::policy::set_default(policy);
    let runtime = ShellRuntime::new_isolated().unwrap();
    let root = staged.wrapper("mods/composer", setup);
    let window = cx.add_window(move |window, cx| {
        let root = runtime.try_load(&root, window, cx).unwrap();
        Framed(root, framed.unwrap_or_else(|| Rc::new(Cell::new(600.0))))
    });
    let mut visual = VisualTestContext::from_window(*window.deref(), cx);
    for _ in 0..2 {
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        visual.run_until_parked();
    }
    (visual, window, calls)
}

fn draw(visual: &mut VisualTestContext) {
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear(cx));
    visual.run_until_parked();
}

/// The jump-to-latest pill is centred over the conversation and sits 12 px above its panel's bottom,
/// with a down arrow before its words.
#[gpui::test]
fn the_jump_latest_pill_is_centred_above_the_composer(cx: &mut TestAppContext) {
    use gpui::{point, px, Modifiers, ScrollDelta, ScrollWheelEvent, TouchPhase};
    // A conversation that fits is at its end, so the pill needs a long one scrolled up.
    let values = composer_values(composer_conversation(80));
    let (mut visual, window, calls) = mount_composer(cx, values, "", None);
    let at = visual
        .debug_bounds("composer-conversation")
        .expect("the conversation is laid out")
        .center();
    visual.simulate_event(ScrollWheelEvent {
        position: at,
        delta: ScrollDelta::Pixels(point(px(0.0), px(400.0))),
        modifiers: Modifiers::default(),
        touch_phase: TouchPhase::Moved,
    });
    draw(&mut visual);
    let pill = visual
        .debug_bounds("composer-jump-latest")
        .expect("the pill is laid out");
    let list = visual
        .debug_bounds("composer-conversation")
        .expect("the conversation is laid out");
    let panel = visual
        .debug_bounds("composer-panel")
        .expect("the panel is laid out");
    let slack = gpui::px(1.0);
    assert!(
        (pill.center().x - list.center().x).abs() <= slack,
        "the pill is centred over the conversation: pill {pill:?}, list {list:?}"
    );
    assert!(
        (panel.bottom() - pill.bottom() - gpui::px(12.0)).abs() <= slack,
        "the pill ends 12 px above the panel: pill {pill:?}, panel {panel:?}"
    );
    drop(visual);
    cx.run_until_parked();
    window.update(cx, |_, window, _| window.remove_window()).ok();
    cx.run_until_parked();
    assert!(calls.borrow().unexpected.is_empty(), "{:?}", calls.borrow().unexpected);
}

/// Scrolling up shows the pill; scrolling back to within 24 px of the end sticks the conversation
/// again and hides it.
#[gpui::test]
fn scrolling_up_shows_the_pill_and_the_end_hides_it(cx: &mut TestAppContext) {
    use gpui::{point, px, Modifiers, ScrollDelta, ScrollWheelEvent, TouchPhase};
    let values = composer_values(composer_conversation(80));
    let (mut visual, window, calls) = mount_composer(cx, values, "", None);
    assert!(
        visual.debug_bounds("composer-jump-latest").is_none(),
        "a stuck conversation has no pill"
    );
    let at = visual
        .debug_bounds("composer-conversation")
        .expect("the conversation is laid out")
        .center();
    visual.simulate_event(ScrollWheelEvent {
        position: at,
        delta: ScrollDelta::Pixels(point(px(0.0), px(200.0))),
        modifiers: Modifiers::default(),
        touch_phase: TouchPhase::Moved,
    });
    draw(&mut visual);
    assert!(
        visual.debug_bounds("composer-jump-latest").is_some(),
        "scrolling up shows the pill"
    );
    for _ in 0..40 {
        visual.simulate_event(ScrollWheelEvent {
            position: at,
            delta: ScrollDelta::Pixels(point(px(0.0), px(-200.0))),
            modifiers: Modifiers::default(),
            touch_phase: TouchPhase::Moved,
        });
    }
    draw(&mut visual);
    assert!(
        visual.debug_bounds("composer-jump-latest").is_none(),
        "scrolling back to the end hides the pill"
    );
    drop(visual);
    cx.run_until_parked();
    window.update(cx, |_, window, _| window.remove_window()).ok();
    cx.run_until_parked();
    assert!(calls.borrow().unexpected.is_empty(), "{:?}", calls.borrow().unexpected);
}

/// The mode word, the context ring and the limit words left the box; the box, attach, model and Send stay.
#[gpui::test]
fn the_box_has_no_mode_ring_or_limit(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut values = composer_mode_values("default");
    values.get_mut("lead").unwrap()["usage"] = json!({
        "context_tokens": 400000,
        "context_window": 1000000,
        "compact_at": 444000,
        "five_hour": {"used_percentage": 82, "resets_at": 4102444800i64},
        "seven_day": null
    });
    let tree = describe_settled(cx, &staged, "mods/composer", values, None).unwrap();
    for id in ["composer-box", "composer-attach", "composer-model", "composer-send"] {
        assert!(tree.contains(id), "{id}: {tree}");
    }
    for gone in [":id[Str(\"composer-mode\")]", ":id[Str(\"composer-usage\")]", ":id[Str(\"composer-limit\")]", "Ask first", "Context 90%", "5h 82%"] {
        assert!(!tree.contains(gone), "{gone}: {tree}");
    }
}
