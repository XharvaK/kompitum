//! Keyboard behaviour the toolkit gives a mod: the visible-focus style, the
//! focus ring, and keeping the keyboard where it was on a chrome button's
//! press. The ring and the focus-visible style are native, so they are checked
//! in a laid-out window rather than in the description.

use super::*;

use gpui::{Modifiers, MouseButton};
use gpui_shell::focus_ring_test_support;
/// The ring log is one file for the process; these tests share it one at a time.
static RING_LOG: std::sync::Mutex<()> = std::sync::Mutex::new(());

const PROBE: &str = r#"
import { View, div } from 'gpui-kit';
import { v_flex } from 'gpui-base';
import { run } from 'kompitum';
export default class Probe extends View {
  init(_props, cx) {
    this.one = cx.focus_handle();
    this.chosen = cx.focus_handle();
    this.plain = cx.focus_handle();
    this.ringed = cx.focus_handle();
    this.one.focus();
  }
  render() {
    return div().size_full().flex().flex_col().gap(4).children([
      div().id('probe-one').w(200).h(24).bg('#101010')
        .track_focus(this.one).tab_stop(true)
        .focus_visible(style => style.w(120))
        .focus_ring('#00ff00', 4),
      div().id('probe-chosen').w(200).h(24).bg('#202020')
        .track_focus(this.chosen).tab_stop(true).keep_focus(true)
        .focus_visible(style => style.w(120))
        .focus_ring('#ffff00', 4)
        .on_click(() => run('space.next')),
      div().id('probe-plain').w(200).h(24).bg('#303030')
        .track_focus(this.plain).tab_stop(true)
        .on_click(() => run('space.prev')),
      div().id('probe-ringed').w(200).h(24).bg('#404040')
        .track_focus(this.ringed).tab_stop(true)
        .focus_ring('#ff00ff', 6),
      div().id('probe-neighbour').w(200).h(24).bg('#505050'),
      v_flex().id('probe-vflex').w(200).h(24).tab_stop(true).focus_ring('#00ffff', 4)
        .on_click(() => run('space.prev')).child(div().child('vflex')),
    ]);
  }
}
"#;

fn mount_probe(
    cx: &mut TestAppContext,
) -> (
    gpui::WindowHandle<Mounted>,
    gpui::VisualTestContext,
    Rc<RefCell<Calls>>,
) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let calls = Rc::new(RefCell::new(Calls::default()));
    let values = fixture(false, props(), updates("current", "current"));
    let policy = Policy::new()
        .with_storage_path(staged.0.join("store-keyboard.json"))
        .with_capabilities(
            Capabilities::new()
                .storage(true)
                .read_roots([staged.0.clone()]),
        )
        .with_host_module(module(values, calls.clone()))
        .unwrap();
    gpui_shell::policy::set_default(policy);
    let log = std::env::temp_dir().join(format!(
        "kompitum-focus-rings-{}.log",
        std::process::id()
    ));
    // SAFETY: every test that reads the log holds RING_LOG, so no other test
    // thread is reading the environment while it is written.
    unsafe { std::env::set_var("KOMPITUM_FOCUS_RING_LOG", &log) };
    let root = staged.0.join("mods/probe");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("mod.json"), r#"{ "place": "none" }"#).unwrap();
    std::fs::write(root.join("main.js"), PROBE).unwrap();
    let runtime = ShellRuntime::new_isolated().unwrap();
    let window = cx.add_window(|window, cx| Mounted(runtime.try_load(&root, window, cx).unwrap()));
    let mut visual = VisualTestContext::from_window(*window.deref(), cx);
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear(cx));
    visual.run_until_parked();
    (window, visual, calls)
}

fn draw(visual: &mut gpui::VisualTestContext) {
    focus_ring_test_support::reset();
    visual.update(|window, cx| {
        window.refresh();
        window.draw(cx).clear(cx)
    });
    visual.run_until_parked();
}

fn width(visual: &mut gpui::VisualTestContext, id: &'static str) -> f32 {
    f32::from(visual.debug_bounds(id).unwrap_or_else(|| panic!("{id} is laid out")).size.width)
}

fn click(visual: &mut gpui::VisualTestContext, id: &'static str) {
    let at = visual
        .debug_bounds(id)
        .unwrap_or_else(|| panic!("{id} is laid out"))
        .center();
    visual.simulate_mouse_move(at, None, Modifiers::default());
    visual.simulate_mouse_down(at, MouseButton::Left, Modifiers::default());
    visual.simulate_mouse_up(at, MouseButton::Left, Modifiers::default());
    visual.run_until_parked();
    draw(visual);
}

fn rings_named(identity: &str) -> usize {
    focus_ring_test_support::painted()
        .iter()
        .filter(|ring| ring.identity.contains(identity))
        .count()
}

/// A whole key press: `simulate_keystrokes` sends the key down alone, and
/// GPUI's keyboard click is on the release.
fn press_key(visual: &mut gpui::VisualTestContext, key: &str) {
    let keystroke = gpui::Keystroke::parse(key).unwrap();
    visual.simulate_event(gpui::KeyDownEvent {
        keystroke: keystroke.clone(),
        is_held: false,
        prefer_character_input: false,
    });
    visual.simulate_event(gpui::KeyUpEvent { keystroke });
    visual.run_until_parked();
}

/// Keyboard input reaches the window and goes to a named control without
/// relying on the Tab order: click it, then press a key.
fn keyboard_focus_on(visual: &mut gpui::VisualTestContext, id: &'static str) {
    click(visual, id);
    visual.simulate_keystrokes("space");
    visual.run_until_parked();
    draw(visual);
}

/// The state style follows keyboard focus, not a click: its declaration reads
/// as a width while a focused element is drawn, and only for that focus.
#[gpui::test]
fn focus_visible_follows_keyboard_focus_and_not_a_click(cx: &mut TestAppContext) {
    let _guard = RING_LOG.lock().unwrap_or_else(|poison| poison.into_inner());
    let (window, mut visual, calls) = mount_probe(cx);
    draw(&mut visual);
    assert!(focus_ring_test_support::painted().is_empty(), "a focus set in init paints no ring");
    visual.simulate_keystrokes("tab");
    visual.run_until_parked();
    draw(&mut visual);
    assert_eq!(width(&mut visual, "probe-chosen"), 120.0, "Tab focuses the second stop");
    assert_eq!(rings_named("probe-chosen"), 1, "Tab's focus is visible, so it has a ring");
    click(&mut visual, "probe-plain");
    assert_eq!(
        width(&mut visual, "probe-chosen"),
        200.0,
        "a click moves focus away, so the focus-visible style is gone"
    );
    assert!(
        focus_ring_test_support::painted().is_empty(),
        "a click paints no ring: {:?}",
        focus_ring_test_support::painted()
    );
    let calls = calls.borrow();
    assert!(calls.unexpected.is_empty(), "{:?}", calls.unexpected);
    drop(calls);
    drop(visual);
    cx.run_until_parked();
    window.update(cx, |_, window, _| window.remove_window()).ok();
    cx.run_until_parked();
}

/// The ring appears on a keyboard-focused element, disappears on a click, and
/// changes neither the element's size nor its neighbour's place.
#[gpui::test]
fn the_focus_ring_appears_for_the_keyboard_only_and_moves_nothing(cx: &mut TestAppContext) {
    let _guard = RING_LOG.lock().unwrap_or_else(|poison| poison.into_inner());
    let (window, mut visual, calls) = mount_probe(cx);
    keyboard_focus_on(&mut visual, "probe-ringed");
    assert_eq!(rings_named("probe-ringed"), 1, "the focused element has its ring");
    let before = (
        visual.debug_bounds("probe-ringed").unwrap(),
        visual.debug_bounds("probe-neighbour").unwrap().origin.y,
    );
    click(&mut visual, "probe-plain");
    draw(&mut visual);
    assert_eq!(rings_named("probe-ringed"), 0, "a click paints no ring");
    keyboard_focus_on(&mut visual, "probe-ringed");
    assert_eq!(rings_named("probe-ringed"), 1, "the ring is back with the keyboard");
    let after = (
        visual.debug_bounds("probe-ringed").unwrap(),
        visual.debug_bounds("probe-neighbour").unwrap().origin.y,
    );
    assert_eq!(before, after, "the ring is outside the layout, so nothing moved");
    let ring = focus_ring_test_support::painted()
        .into_iter()
        .find(|ring| ring.identity.contains("probe-ringed"))
        .unwrap();
    assert_eq!(ring.bounds, after.0.inset(gpui::px(-3.0)), "2 px wide, 1 px outside");
    assert_eq!(ring.radius, gpui::px(6.0));
    let calls = calls.borrow();
    assert!(calls.unexpected.is_empty(), "{:?}", calls.unexpected);
    drop(calls);
    drop(visual);
    cx.run_until_parked();
    window.update(cx, |_, window, _| window.remove_window()).ok();
    cx.run_until_parked();
}

/// A chrome button's press leaves the previous focus in place and still calls
/// its click.
#[gpui::test]
fn keep_focus_leaves_the_previous_focus_and_still_clicks(cx: &mut TestAppContext) {
    let _guard = RING_LOG.lock().unwrap_or_else(|poison| poison.into_inner());
    let (window, mut visual, calls) = mount_probe(cx);
    keyboard_focus_on(&mut visual, "probe-ringed");
    assert_eq!(rings_named("probe-ringed"), 1);
    click(&mut visual, "probe-chosen");
    draw(&mut visual);
    assert_eq!(
        focus_ring_test_support::painted().len(),
        0,
        "a mouse click shows no ring anywhere"
    );
    visual.simulate_keystrokes("space");
    visual.run_until_parked();
    draw(&mut visual);
    assert_eq!(rings_named("probe-ringed"), 1, "the ringed element keeps focus");
    assert_eq!(rings_named("probe-chosen"), 0, "the pressed button took none");
    let calls = calls.borrow();
    assert!(
        calls
            .effects
            .iter()
            .any(|(name, args)| name == "run" && args.first() == Some(&json!("space.next"))),
        "the click still fired: {:?}",
        calls.effects
    );
    assert!(calls.unexpected.is_empty(), "{:?}", calls.unexpected);
    drop(calls);
    drop(visual);
    cx.run_until_parked();
    window.update(cx, |_, window, _| window.remove_window()).ok();
    cx.run_until_parked();
}

/// A top bar button and the composer in one window: the composer's own mod and
/// the shipped top bar, both as the app mounts them.
const CHROME_PROBE: &str = r#"
import { View, div } from 'gpui-kit';
import { v_flex } from 'gpui-base';
import TopBar from './topbar.js';
import Composer from './composer/main.js';
export default class ChromeProbe extends View {
  init(_props, cx) {
    this.top = cx.new(TopBar);
    this.composer = cx.new(Composer, { terminal: 't1' });
  }
  render() {
    return v_flex().size_full().children([
      div().h(36).w_full().flex_none().child(this.top),
      div().flex_1().min_h(0).child(this.composer),
    ]);
  }
}
"#;

/// The composer's two boxes need a lead that waits, with a mode word; the
/// topbar needs the same window values, and both are stubbed by the fixture.
fn composer_values() -> BTreeMap<String, Value> {
    let mut values = fixture(true, props(), updates("current", "current"));
    values.insert("knownAgents".into(), agent_catalog());
    values.insert(
        "person".into(),
        json!({"lead": {"program": "claude", "model": "sonnet", "folder": "/fixture/project"}}),
    );
    values.insert(
        "lead".into(),
        json!({"state": "waiting", "ask": null, "started": true, "permission_mode": "default", "mode_guessed": false}),
    );
    values
}

fn mount_chrome(
    cx: &mut TestAppContext,
) -> (
    gpui::WindowHandle<Mounted>,
    gpui::VisualTestContext,
    Rc<RefCell<Calls>>,
) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let calls = Rc::new(RefCell::new(Calls::default()));
    let policy = Policy::new()
        .with_storage_path(staged.0.join("store-chrome.json"))
        .with_capabilities(
            Capabilities::new()
                .storage(true)
                .read_roots([staged.0.clone()]),
        )
        .with_host_module(module(composer_values(), calls.clone()))
        .unwrap();
    gpui_shell::policy::set_default(policy);
    let root = staged.0.join("mods/chrome-probe");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("mod.json"), r#"{ "place": "none" }"#).unwrap();
    // The top bar and the composer travel into this mod's folder: a relative
    // import may not leave the application directory.
    std::fs::copy(
        staged.0.join("mods/topbar/main.js"),
        root.join("topbar.js"),
    )
    .unwrap();
    let composer = root.join("composer");
    std::fs::create_dir_all(&composer).unwrap();
    for entry in std::fs::read_dir(staged.0.join("mods/composer")).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_file() {
            std::fs::copy(entry.path(), composer.join(entry.file_name())).unwrap();
        }
    }
    std::fs::write(root.join("main.js"), CHROME_PROBE).unwrap();
    let runtime = ShellRuntime::new_isolated().unwrap();
    let window = cx.add_window(|window, cx| Mounted(runtime.try_load(&root, window, cx).unwrap()));
    let mut visual = VisualTestContext::from_window(*window.deref(), cx);
    visual.run_until_parked();
    draw(&mut visual);
    (window, visual, calls)
}

/// Clicks the composer's text area: the upper part of its box, above the row
/// of buttons.
fn click_composer_box(visual: &mut gpui::VisualTestContext) {
    let at = visual.debug_bounds("composer-box").unwrap();
    let at = gpui::point(at.origin.x + at.size.width / 2.0, at.origin.y + gpui::px(20.0));
    visual.simulate_mouse_move(at, None, Modifiers::default());
    visual.simulate_mouse_down(at, MouseButton::Left, Modifiers::default());
    visual.simulate_mouse_up(at, MouseButton::Left, Modifiers::default());
    visual.run_until_parked();
}

/// The composer's sends, in order, from the `kom send` calls the fixture saw.
fn sent_bodies(calls: &Rc<RefCell<Calls>>) -> Vec<String> {    calls
        .borrow()
        .effects
        .iter()
        .filter(|(name, _)| name == "kom")
        .filter_map(|(_, args)| args.first()?.as_array().cloned())
        .filter(|argv| argv.first().and_then(Value::as_str) == Some("send"))
        .filter_map(|argv| argv.get(2).and_then(Value::as_str).map(str::to_owned))
        .collect()
}

/// With the composer box focused, Send, the mode word and a top bar button all
/// leave the keyboard in the box: the next typing still reaches it.
#[gpui::test]
fn chrome_buttons_leave_the_composer_box_focused(cx: &mut TestAppContext) {
    let (window, mut visual, calls) = mount_chrome(cx);
    click_composer_box(&mut visual);
    for (button, text) in [
        ("composer-send", "after send"),
        ("composer-mode", "after mode"),
        ("panel-toggle", "after top bar"),
    ] {
        visual.simulate_input("x");
        visual.run_until_parked();
        if button == "composer-send" {
            // Send needs something in the box; the "x" it sends is the click's
            // own message, and the typing after it is what this test reads.
            visual.simulate_keystrokes("enter");
            visual.run_until_parked();
        }
        click(&mut visual, button);
        visual.simulate_input(text);
        visual.run_until_parked();
        visual.simulate_keystrokes("enter");
        visual.run_until_parked();
    }
    let bodies = sent_bodies(&calls);
    for text in ["x", "after send", "after mode", "after top bar"] {
        assert!(
            bodies.iter().any(|body| body.contains(text)),
            "{text:?} reached the box after the buttons: {bodies:?}"
        );
    }
    let calls = calls.borrow();
    assert!(calls.unexpected.is_empty(), "{:?}", calls.unexpected);
    drop(calls);
    drop(visual);
    cx.run_until_parked();
    window.update(cx, |_, window, _| window.remove_window()).ok();
    cx.run_until_parked();
}

/// The chrome buttons that are clicked with a mouse while a terminal or the
/// composer holds the keyboard ask for `keep_focus` in their own mods.
#[gpui::test]
fn chrome_and_composer_buttons_keep_focus(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    for (entry, ids) in [
        (
            "mods/topbar",
            vec!["topbar-limits", "topbar-bell", "panel-toggle"],
        ),
        (
            "mods/composer",
            vec!["composer-send", "composer-attach", "composer-mode", "composer-model"],
        ),
        ("mods/spaces", vec!["space-new"]),
        ("mods/agents", vec!["ag-sort"]),
        ("mods/bell", vec!["bell-note-0-do"]),
    ] {
        let values = if entry == "mods/composer" {
            composer_values()
        } else {
            fixture(true, props(), updates("current", "current"))
        };
        let tree = describe_settled(cx, &staged, entry, values, None).unwrap();
        for id in ids {
            let line = tree
                .lines()
                .find(|line| line.contains(&format!(":id[Str(\"{id}\")]")))
                .unwrap_or_else(|| panic!("{entry}: no {id}: {tree}"));
            assert!(line.contains(":keep_focus"), "{entry}/{id}: {line}");
        }
    }
}

/// Mounts one shipped mod as the window, for the keyboard checks that need a
/// real Tab order: the composer's card, and a Settings page.
fn mount_mod(
    cx: &mut TestAppContext,
    staged: &Staged,
    entry: &str,
    values: BTreeMap<String, Value>,
    setup: &str,
) -> (
    gpui::WindowHandle<Mounted>,
    gpui::VisualTestContext,
    Rc<RefCell<Calls>>,
) {
    let calls = Rc::new(RefCell::new(Calls::default()));
    let policy = Policy::new()
        .with_storage_path(staged.0.join(format!("store-keys-{}.json", std::process::id())))
        .with_capabilities(
            Capabilities::new()
                .storage(true)
                .read_roots([staged.0.clone()]),
        )
        .with_host_module(module(values, calls.clone()))
        .unwrap();
    gpui_shell::policy::set_default(policy);
    let log = std::env::temp_dir().join(format!(
        "kompitum-focus-rings-{}.log",
        std::process::id()
    ));
    // SAFETY: every test that reads the log holds RING_LOG, so no other test
    // thread is reading the environment while it is written.
    unsafe { std::env::set_var("KOMPITUM_FOCUS_RING_LOG", &log) };
    let root = staged.wrapper(entry, setup);
    let runtime = ShellRuntime::new_isolated().unwrap();
    let window = cx.add_window(|window, cx| Mounted(runtime.try_load(&root, window, cx).unwrap()));
    let mut visual = VisualTestContext::from_window(*window.deref(), cx);
    visual.run_until_parked();
    draw(&mut visual);
    (window, visual, calls)
}

/// Tabs until a ring with `wanted` in its id is painted, and says whether it
/// was reached. Each Tab is one frame, so the ring names the control that has
/// the keyboard now.
fn tab_to(visual: &mut gpui::VisualTestContext, wanted: &str, limit: usize) -> bool {
    for _ in 0..limit {
        visual.simulate_keystrokes("tab");
        visual.run_until_parked();
        draw(visual);
        if focus_ring_test_support::painted().iter().any(|ring| ring.identity.contains(wanted)) {
            return true;
        }
    }
    false
}

fn card_values() -> BTreeMap<String, Value> {
    let mut values = composer_with_ask(
        "permission",
        json!({"tool_name": "Bash", "tool_input": {"command": "ls"}}),
    );
    values.insert(
        "person".into(),
        json!({"lead": {"program": "claude", "model": "sonnet", "folder": "/fixture/project"}}),
    );
    values.insert(
        "lead".into(),
        json!({"state": "asking", "started": true, "ask": {"id": "a1", "terminal": "t1", "kind": "permission", "input": {"tool_name": "Bash", "tool_input": {"command": "ls"}}}}),
    );
    values
}

/// Tab reaches the card's controls, the ring follows, and Enter presses the
/// focused one: Allow answers the lead's ask.
#[gpui::test]
fn tab_and_enter_press_a_cards_controls(cx: &mut TestAppContext) {
    let _guard = RING_LOG.lock().unwrap_or_else(|poison| poison.into_inner());
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let (window, mut visual, calls) = mount_mod(
        cx,
        &staged,
        "mods/composer",
        card_values(),
        "this.askId = 'a1'; this.askSince = 0;",
    );
    assert!(
        tab_to(&mut visual, "composer-allow", 12),
        "Tab reaches Allow: {:?}",
        focus_ring_test_support::painted()
    );
    visual.simulate_keystrokes("tab");
    visual.run_until_parked();
    draw(&mut visual);
    let next: Vec<String> = focus_ring_test_support::painted().iter().map(|ring| ring.identity.clone()).collect();
    assert!(
        !next.is_empty() && !next.iter().any(|id| id.contains("composer-allow")),
        "Tab moves to the next control, and the ring follows it: {next:?}"
    );
    visual.simulate_keystrokes("shift-tab");
    visual.run_until_parked();
    draw(&mut visual);
    assert!(
        focus_ring_test_support::painted().iter().any(|ring| ring.identity.contains("composer-allow")),
        "Shift+Tab comes back to Allow"
    );
    press_key(&mut visual, "space");
    let calls = calls.borrow();
    assert!(
        calls.effects.iter().any(|(name, args)| {
            name == "run"
                && args.first() == Some(&json!("lead.answer"))
                && args.get(1) == Some(&json!({"id": "a1", "answer": {"action": "allow"}}))
        }),
        "Space pressed Allow: {:?}",
        calls.effects
    );
    assert!(calls.unexpected.is_empty(), "{:?}", calls.unexpected);
    drop(calls);
    drop(visual);
    cx.run_until_parked();
    window.update(cx, |_, window, _| window.remove_window()).ok();
    cx.run_until_parked();
}

/// Enter and Space press a `v_flex` the way they press an `h_flex`: GPUI's
/// keyboard click on a tab stop the shell made for the focus ring.
#[gpui::test]
fn enter_presses_a_vflex_control(cx: &mut TestAppContext) {
    let _guard = RING_LOG.lock().unwrap_or_else(|poison| poison.into_inner());
    let (window, mut visual, calls) = mount_probe(cx);
    click(&mut visual, "probe-vflex");
    draw(&mut visual);
    assert_eq!(rings_named("probe-vflex"), 0, "the click shows no ring");
    press_key(&mut visual, "enter");
    let calls = calls.borrow();
    assert!(
        calls
            .effects
            .iter()
            .any(|(name, args)| name == "run" && args.first() == Some(&json!("space.prev"))),
        "Enter pressed it: {:?}",
        calls.effects
    );
    assert!(calls.unexpected.is_empty(), "{:?}", calls.unexpected);
    drop(calls);
    drop(visual);
    cx.run_until_parked();
    window.update(cx, |_, window, _| window.remove_window()).ok();
    cx.run_until_parked();
}

/// Tab focus, not a click, still lets Enter press a control: the keyboard
/// click GPUI gives a focused element does not depend on how it was focused.
#[gpui::test]
fn enter_presses_a_control_reached_by_tab(cx: &mut TestAppContext) {
    let _guard = RING_LOG.lock().unwrap_or_else(|poison| poison.into_inner());
    let (window, mut visual, calls) = mount_probe(cx);
    assert!(tab_to(&mut visual, "probe-vflex", 10), "{:?}", focus_ring_test_support::painted());
    press_key(&mut visual, "space");
    let calls = calls.borrow();
    assert!(
        calls
            .effects
            .iter()
            .any(|(name, args)| name == "run" && args.first() == Some(&json!("space.prev"))),
        "Enter pressed it: {:?}",
        calls.effects
    );
    assert!(calls.unexpected.is_empty(), "{:?}", calls.unexpected);
    drop(calls);
    drop(visual);
    cx.run_until_parked();
    window.update(cx, |_, window, _| window.remove_window()).ok();
    cx.run_until_parked();
}

/// Settings: Tab reaches a tile with its ring, and Enter presses it, writing
/// the picked appearance.
#[gpui::test]
fn tab_and_enter_press_a_settings_control(cx: &mut TestAppContext) {    let _guard = RING_LOG.lock().unwrap_or_else(|poison| poison.into_inner());
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut p = props();
    p["section"] = json!("appearance");
    let mut values = fixture(true, p, updates("current", "current"));
    values.insert("person".into(), json!({}));
    let (window, mut visual, calls) =
        mount_mod(cx, &staged, "frame/settings", values, "this.searchInput.focus();");
    assert!(
        tab_to(&mut visual, "st-ap-Light", 80),
        "Tab reaches the Light tile: {:?}",
        focus_ring_test_support::painted()
    );
    // The next tile is the one this test presses: the first was only reached.
    assert!(
        tab_to(&mut visual, "st-ap-Dark", 4),
        "Tab reaches the next tile: {:?}",
        focus_ring_test_support::painted()
    );
    press_key(&mut visual, "enter");
    visual.run_until_parked();
    let calls = calls.borrow();
    let written: Vec<&Value> = calls
        .effects
        .iter()
        .filter(|(name, _)| name == "writeTextFile")
        .filter_map(|(_, args)| args.get(1))
        .collect();
    assert!(
        written.iter().any(|text| {
            text.as_str()
                .is_some_and(|text| text.contains("\"appearance\": \"dark\""))
        }),
        "Enter pressed the Dark tile: {written:?}"
    );
    assert!(calls.unexpected.is_empty(), "{:?}", calls.unexpected);
    drop(calls);
    drop(visual);
    cx.run_until_parked();
    window.update(cx, |_, window, _| window.remove_window()).ok();
    cx.run_until_parked();
}

/// The card closes as the hook's reply would close it: the box takes the
/// keyboard back, and the next message typed reaches it.
#[gpui::test]
fn the_composer_box_takes_the_keyboard_after_a_card_closes(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut values = card_values();
    values.insert(
        "leadAfterAnswer".into(),
        json!({"state": "waiting", "started": true, "ask": null, "permission_mode": "default"}),
    );
    let (window, mut visual, calls) = mount_mod(
        cx,
        &staged,
        "mods/composer",
        values,
        "this.askId = 'a1'; this.askSince = 0;",
    );
    press_key(&mut visual, "enter");
    visual.run_until_parked();
    visual.simulate_input("back home");
    visual.run_until_parked();
    visual.simulate_keystrokes("enter");
    visual.run_until_parked();
    let bodies = sent_bodies(&calls);
    assert!(
        bodies.iter().any(|body| body.contains("back home")),
        "the box had the keyboard after the card: {bodies:?}"
    );
    let calls = calls.borrow();
    assert!(calls.unexpected.is_empty(), "{:?}", calls.unexpected);
    drop(calls);
    drop(visual);
    cx.run_until_parked();
    window.update(cx, |_, window, _| window.remove_window()).ok();
    cx.run_until_parked();
}

/// The model menu closes on its trigger: the box takes the keyboard back, and
/// the next message typed reaches it.
#[gpui::test]
fn the_composer_box_takes_the_keyboard_after_the_model_menu_closes(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let (window, mut visual, calls) = mount_mod(
        cx,
        &staged,
        "mods/composer",
        composer_values(),
        "this.modelOpen = true; this.text.focus();",
    );
    click(&mut visual, "composer-model");
    visual.run_until_parked();
    visual.simulate_input("after menu");
    visual.run_until_parked();
    visual.simulate_keystrokes("enter");
    visual.run_until_parked();
    let bodies = sent_bodies(&calls);
    assert!(
        bodies.iter().any(|body| body.contains("after menu")),
        "the box had the keyboard after the menu: {bodies:?}"
    );
    let calls = calls.borrow();
    assert!(calls.unexpected.is_empty(), "{:?}", calls.unexpected);
    drop(calls);
    drop(visual);
    cx.run_until_parked();
    window.update(cx, |_, window, _| window.remove_window()).ok();
    cx.run_until_parked();
}

/// Icon-only chrome controls name themselves with their own tooltip.
#[gpui::test]
fn icon_buttons_carry_their_tooltip_as_their_name(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    for (entry, id, label) in [
        ("mods/topbar", "topbar-limits", "Limits"),
        ("mods/topbar", "panel-toggle", "Show the side panel"),
        ("mods/workspace-chrome", "settings", "Settings"),
        ("mods/bell", "bell-note-0-do", "Open"),
    ] {
        let mut p = props();
        if entry == "mods/workspace-chrome" {
            p["footer"] = json!(true);
        }
        let tree = describe_settled(
            cx,
            &staged,
            entry,
            fixture(true, p, updates("current", "current")),
            None,
        )
        .unwrap();
        let line = tree
            .lines()
            .find(|line| line.contains(&format!(":id[Str(\"{id}\")]")))
            .unwrap_or_else(|| panic!("{entry}: no {id}: {tree}"));
        assert!(
            line.contains(&format!(":accessibility_label[Str(\"{label}\")]")),
            "{entry}/{id} should announce {label:?}: {line}"
        );
    }
}

/// The composer's `/` list is a list_box of options: the highlighted row is
/// announced as selected, and the composite claims it as the active one.
#[gpui::test]
fn the_command_list_announces_its_highlighted_option(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut values = composer_values();
    values.insert(
        "leadCommands".into(),
        json!([
            {"name": "compact", "hint": "", "description": "Tidy up", "source": "built-in"},
            {"name": "clear", "hint": "", "description": "Clear", "source": "built-in"}
        ]),
    );
    let tree = describe_settled(
        cx,
        &staged,
        "mods/composer",
        values,
        Some("this.text.set_value('/'); this.text.set_selected_range({start: 1, end: 1}); this.refreshList(cx); this.text.focus();"),
    )
    .unwrap();
    assert!(tree.contains(":role[Str(\"list_box\")]"), "the list is a list_box: {tree}");
    assert!(tree.contains("composer-list-0"), "{tree}");
    assert!(tree.contains(":role[Str(\"list_box_option\")]"), "{tree}");
    let lines: Vec<&str> = tree.lines().collect();
    let selected = |index: usize| {
        let at = lines
            .iter()
            .position(|line| line.contains(&format!("composer-list-{index}")))
            .unwrap_or_else(|| panic!("no row {index}: {tree}"));
        lines[at]
    };
    assert!(
        selected(0).contains(":aria_selected[Bool(true)]"),
        "the first row is highlighted: {}",
        selected(0)
    );
    assert!(
        selected(0).contains(":aria_active_descendant"),
        "the focused box's active descendant is the highlighted row: {}",
        selected(0)
    );
    assert!(
        !selected(1).contains(":aria_selected[Bool(true)]"),
        "the other row is not: {}",
        selected(1)
    );
}

/// Three agents in one space: the focused one sits between the other two, so
/// starting on the selected row and moving from it are both visible.
fn agents_for_keys() -> BTreeMap<String, Value> {
    let mut values = fixture(true, props(), updates("current", "current"));
    let program = values["agents"][0]["program"].clone();
    let agent = |address: &str, pane: &str, status: &str, focused: bool, seq: i64| {
        json!({
            "address": address, "pane": pane, "space": "s1", "space_name": "Local",
            "tab": "tab1", "tab_index": 1, "tab_name": "Build", "status": status,
            "app": "pi", "program": program.clone(), "title": address, "seq": seq, "focused": focused
        })
    };
    values.insert(
        "agents".into(),
        json!([
            agent("t2", "p2", "idle", false, 3),
            agent("t1", "p1", "working", true, 2),
            agent("t3", "p3", "idle", false, 1)
        ]),
    );
    values.insert(
        "focused".into(),
        json!({
            "space": "s1", "tab": "tab1", "pane": "p1", "terminal": "t1",
            "agent": "t1", "shown": null, "cwd": "/fixture/project"
        }),
    );
    values.insert("sharedValues".into(), json!({"agents.sort": "grouped"}));
    values
}

/// The agents list is a list of rows for the keyboard: the ring starts on the
/// selected agent, Up and Down move it without wrapping, and Enter opens the
/// row the way its click does.
#[gpui::test]
fn arrows_move_the_agents_highlight_and_enter_opens_the_row(cx: &mut TestAppContext) {
    let _guard = RING_LOG.lock().unwrap_or_else(|poison| poison.into_inner());
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let (window, mut visual, calls) = mount_mod(
        cx,
        &staged,
        "mods/agents",
        agents_for_keys(),
        "this.handle.focus();",
    );
    draw(&mut visual);
    press_key(&mut visual, "left");
    draw(&mut visual);
    assert_eq!(
        rings_named("ag-t1"),
        1,
        "the highlight starts on the selected row: {:?}",
        focus_ring_test_support::painted()
    );
    press_key(&mut visual, "up");
    draw(&mut visual);
    assert_eq!(rings_named("ag-t2"), 1, "Up moves to the row above");
    press_key(&mut visual, "up");
    draw(&mut visual);
    assert_eq!(rings_named("ag-t2"), 1, "Up stops at the first row");
    press_key(&mut visual, "down");
    press_key(&mut visual, "down");
    press_key(&mut visual, "down");
    draw(&mut visual);
    assert_eq!(rings_named("ag-t3"), 1, "Down stops at the last row");
    press_key(&mut visual, "enter");
    let calls = calls.borrow();
    assert!(
        calls.effects.iter().any(|(name, args)| {
            name == "run" && args.first() == Some(&json!("pane.focus")) && args.get(1) == Some(&json!({"pane": "p3"}))
        }),
        "Enter opened the highlighted row: {:?}",
        calls.effects
    );
    assert!(calls.unexpected.is_empty(), "{:?}", calls.unexpected);
    drop(calls);
    drop(visual);
    cx.run_until_parked();
    window.update(cx, |_, window, _| window.remove_window()).ok();
    cx.run_until_parked();
}

/// A panel mod's controls are Tab stops: Tab reaches the first file row and
/// the ring follows it.
#[gpui::test]
fn tab_reaches_a_panel_mods_first_control(cx: &mut TestAppContext) {
    let _guard = RING_LOG.lock().unwrap_or_else(|poison| poison.into_inner());
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let (window, mut visual, calls) = mount_mod(
        cx,
        &staged,
        "mods/files",
        fixture(true, props(), updates("current", "current")),
        "this.handle.focus();",
    );
    assert!(
        tab_to(&mut visual, "file-src", 8),
        "Tab reaches the first file row: {:?}",
        focus_ring_test_support::painted()
    );
    assert_eq!(rings_named("file-src"), 1, "the first control shows the ring");
    let calls = calls.borrow();
    assert!(calls.unexpected.is_empty(), "{:?}", calls.unexpected);
    drop(calls);
    drop(visual);
    cx.run_until_parked();
    window.update(cx, |_, window, _| window.remove_window()).ok();
    cx.run_until_parked();
}

/// A one-choice question is a radio_group of radios; a multi-choice one is a
/// group of checkboxes.
#[gpui::test]
fn question_cards_announce_radio_and_checkbox_options(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let single = json!({"tool_input": {"questions": [
        {"question": "Which database?", "header": "Database", "multiSelect": false,
         "options": [{"label": "Postgres"}, {"label": "SQLite"}]}
    ]}});
    let tree = describe_settled(
        cx,
        &staged,
        "mods/composer",
        composer_with_ask("question", single),
        Some("this.askId = 'a1'; this.askSince = 0;"),
    )
    .unwrap();
    assert!(tree.contains(":role[Str(\"radio_group\")]"), "{tree}");
    assert!(tree.contains(":role[Str(\"radio_button\")]"), "{tree}");
    assert!(!tree.contains(":role[Str(\"check_box\")]"), "{tree}");
    let multi = json!({"tool_input": {"questions": [
        {"question": "Which checks?", "header": "Checks", "multiSelect": true,
         "options": [{"label": "Tests"}, {"label": "Lint"}]}
    ]}});
    let tree = describe_settled(
        cx,
        &staged,
        "mods/composer",
        composer_with_ask("question", multi),
        Some("this.askId = 'a1'; this.askSince = 0;"),
    )
    .unwrap();
    assert!(tree.contains(":role[Str(\"group\")]"), "{tree}");
    assert!(tree.contains(":role[Str(\"check_box\")]"), "{tree}");
    assert!(!tree.contains(":role[Str(\"radio_button\")]"), "{tree}");
}

/// A card's options are a list: Down moves to the next one and Space picks it,
/// and Enter sends the answers.
#[gpui::test]
fn arrow_keys_move_between_a_cards_options_and_space_picks(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let question = json!({"tool_input": {"questions": [
        {"question": "Which database?", "header": "Database", "multiSelect": false,
         "options": [{"label": "Postgres"}, {"label": "SQLite"}]}
    ]}});
    let (window, mut visual, calls) = mount_mod(
        cx,
        &staged,
        "mods/composer",
        composer_with_ask("question", question),
        "this.askId = 'a1'; this.askSince = 0;",
    );
    visual.simulate_keystrokes("down");
    visual.run_until_parked();
    press_key(&mut visual, "space");
    press_key(&mut visual, "enter");
    let calls = calls.borrow();
    assert!(
        calls.effects.iter().any(|(name, args)| {
            name == "run"
                && args.first() == Some(&json!("lead.answer"))
                && args.get(1)
                    == Some(&json!({"id": "a1", "answer": {"action": "answers", "answers": {"Which database?": "SQLite"}}}))
        }),
        "Down then Space picked the second option: {:?}",
        calls.effects
    );
    assert!(calls.unexpected.is_empty(), "{:?}", calls.unexpected);
    drop(calls);
    drop(visual);
    cx.run_until_parked();
    window.update(cx, |_, window, _| window.remove_window()).ok();
    cx.run_until_parked();
}
