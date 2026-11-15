//! Pack 15: polish in the mods outside the composer.
#![allow(unused_imports)]

use super::*;

use gpui::{Modifiers, MouseButton};
use std::time::Duration;

/// The same host calls as `describe_settled`, mounted in a real window so a
/// test can click and type. `decorate` may replace a host function the mod
/// reads live (a layout that changes) or one that fails on purpose.
fn mount_with<F>(
    cx: &mut TestAppContext,
    staged: &Staged,
    entry: &str,
    values: BTreeMap<String, Value>,
    setup: Option<&str>,
    decorate: F,
) -> (
    gpui::WindowHandle<Mounted>,
    VisualTestContext,
    Rc<RefCell<Calls>>,
)
where
    F: FnOnce(HostModule) -> HostModule,
{
    let calls = Rc::new(RefCell::new(Calls::default()));
    mount_with_calls(cx, staged, entry, values, setup, calls, decorate)
}

fn mount_with_calls<F>(
    cx: &mut TestAppContext,
    staged: &Staged,
    entry: &str,
    values: BTreeMap<String, Value>,
    setup: Option<&str>,
    calls: Rc<RefCell<Calls>>,
    decorate: F,
) -> (
    gpui::WindowHandle<Mounted>,
    VisualTestContext,
    Rc<RefCell<Calls>>,
)
where
    F: FnOnce(HostModule) -> HostModule,
{
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let policy = Policy::new()
        .with_storage_path(staged.0.join(format!(
            "store-pack15-{}.json",
            NEXT.fetch_add(1, Ordering::Relaxed)
        )))
        .with_capabilities(
            Capabilities::new()
                .storage(true)
                .read_roots([staged.0.clone()]),
        )
        .with_host_module(decorate(module(values, calls.clone())))
        .unwrap();
    gpui_shell::policy::set_default(policy);
    let runtime = ShellRuntime::new_isolated().unwrap();
    let root = setup
        .map(|s| staged.wrapper(entry, s))
        .unwrap_or_else(|| staged.0.join(entry));
    let window = cx.add_window(|window, cx| Mounted(runtime.try_load(&root, window, cx).unwrap()));
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

fn click(visual: &mut VisualTestContext, id: &'static str) {
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

/// A whole key press: `simulate_keystrokes` sends the key down alone, and
/// GPUI's keyboard click is on the release.
fn press_key(visual: &mut VisualTestContext, key: &str) {
    let keystroke = gpui::Keystroke::parse(key).unwrap();
    visual.simulate_event(gpui::KeyDownEvent {
        keystroke: keystroke.clone(),
        is_held: false,
        prefer_character_input: false,
    });
    visual.simulate_event(gpui::KeyUpEvent { keystroke });
    visual.run_until_parked();
}

fn ran(calls: &Rc<RefCell<Calls>>, command: &str) -> bool {
    calls
        .borrow()
        .effects
        .iter()
        .any(|(name, args)| name == "run" && args.first() == Some(&json!(command)))
}

fn run_count(calls: &Rc<RefCell<Calls>>, command: &str) -> usize {
    calls
        .borrow()
        .effects
        .iter()
        .filter(|(name, args)| name == "run" && args.first() == Some(&json!(command)))
        .count()
}

fn show_node(node: &mut Value, id: &str) {
    if node.get("id").and_then(Value::as_str) == Some(id) {
        node["hidden"] = json!(false);
    }
    if let Some(children) = node.get_mut("children").and_then(Value::as_array_mut) {
        for child in children {
            show_node(child, id);
        }
    }
}

fn main_layout(panel_shown: bool) -> Value {
    let mut layout: Value =
        serde_json::from_str(include_str!("../../../defaults/layouts/main.json")).unwrap();
    if panel_shown {
        show_node(&mut layout, "panel");
    }
    layout
}

fn set_node_active(node: &mut Value, id: &str, active: &str) {
    if node.get("id").and_then(Value::as_str) == Some(id) {
        node["active"] = json!(active);
    }
    if let Some(children) = node.get_mut("children").and_then(Value::as_array_mut) {
        for child in children {
            set_node_active(child, id, active);
        }
    }
}

fn main_layout_surface(panel_shown: bool, surface: &str) -> Value {
    let mut layout = main_layout(panel_shown);
    if panel_shown {
        set_node_active(&mut layout, "panel-body", surface);
    }
    layout
}

fn confirm_for(calls: &Rc<RefCell<Calls>>, command: &str) -> Option<Value> {
    calls
        .borrow()
        .effects
        .iter()
        .filter(|(name, _)| name == "openOverlay")
        .map(|(_, args)| args.first().cloned().unwrap_or(Value::Null))
        .find(|spec| {
            spec.get("mod") == Some(&json!("confirm"))
                && spec.pointer("/props/command/name") == Some(&json!(command))
        })
}

fn shell_program() -> Value {
    json!({"key":"shell", "name":"Shell", "icon":null, "tint":true, "tile":null, "color":null, "shell":true, "agent":false})
}

fn with_shell_pane(mut values: BTreeMap<String, Value>) -> BTreeMap<String, Value> {
    values.get_mut("panes").unwrap()[0]["program"] = shell_program();
    values.get_mut("panes").unwrap()[0]["status"] = Value::Null;
    values
}

fn finish(
    cx: &mut TestAppContext,
    window: gpui::WindowHandle<Mounted>,
    visual: VisualTestContext,
) {
    drop(visual);
    cx.run_until_parked();
    window.update(cx, |_, window, _| window.remove_window()).ok();
    cx.run_until_parked();
}


/// An agent with a live sub-agent draws the parent's row and the sub-agent's.
#[gpui::test]
fn agents_with_a_running_sub_agent_draw_both_rows(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut values = fixture(true, props(), updates("current", "current"));
    let program = values["agents"][0]["program"].clone();
    let parent = values["terminals"][0].clone();
    let mut child = parent.clone();
    child["address"] = json!("t9");
    child["name"] = json!("Explore");
    child["title"] = json!("Explore");
    child["parent"] = json!("t1");
    child["status"] = json!("working");
    child["program"] = program;
    values.get_mut("terminals").unwrap().as_array_mut().unwrap().push(child);
    let tree = describe_settled(cx, &staged, "mods/agents", values, None).unwrap();
    assert!(tree.contains(r#":id[Str("ag-t1")]"#), "the parent row is drawn: {tree}");
    assert!(tree.contains(r#":id[Str("sub-t9")]"#), "the sub-agent row is drawn: {tree}");
}

/// A tab closes a plain shell at once, and asks before closing one that runs
/// an agent.
#[gpui::test]
fn tabs_ask_before_closing_a_tab_that_runs_something(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let (window, mut visual, calls) = mount_with(
        cx,
        &staged,
        "mods/tabs",
        with_shell_pane(fixture(true, props(), updates("current", "current"))),
        None,
        |m| m,
    );
    click(&mut visual, "close-tab1");
    assert!(ran(&calls, "tab.close"), "a shell-only tab closes at once");
    finish(cx, window, visual);

    let (window, mut visual, calls) = mount_with(
        cx,
        &staged,
        "mods/tabs",
        fixture(true, props(), updates("current", "current")),
        None,
        |m| m,
    );
    click(&mut visual, "close-tab1");
    assert!(!ran(&calls, "tab.close"), "a running tab is not closed at once");
    let ask = confirm_for(&calls, "tab.close").expect("the running tab asks first");
    assert_eq!(ask.pointer("/props/title"), Some(&json!("Close terminal?")));
    assert_eq!(ask.pointer("/props/detail"), Some(&json!("Build is still running.")));
    finish(cx, window, visual);
}

/// A pane header closes a shell at once, and asks before closing a pane whose
/// terminal runs an agent.
#[gpui::test]
fn pane_headers_ask_before_closing_a_pane_that_runs_something(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let (window, mut visual, calls) = mount_with(
        cx,
        &staged,
        "mods/pane",
        with_shell_pane(fixture(true, props(), updates("current", "current"))),
        Some("this.hovered = true;"),
        |m| m,
    );
    click(&mut visual, "ph-x-p1");
    assert!(ran(&calls, "pane.close"), "a shell-only pane closes at once");
    finish(cx, window, visual);

    let (window, mut visual, calls) = mount_with(
        cx,
        &staged,
        "mods/pane",
        fixture(true, props(), updates("current", "current")),
        Some("this.hovered = true;"),
        |m| m,
    );
    click(&mut visual, "ph-x-p1");
    assert!(!ran(&calls, "pane.close"), "a running pane is not closed at once");
    let ask = confirm_for(&calls, "pane.close").expect("the running pane asks first");
    assert_eq!(ask.pointer("/props/title"), Some(&json!("Close terminal?")));
    assert_eq!(ask.pointer("/props/detail"), Some(&json!("Build is still running.")));
    finish(cx, window, visual);
}

/// Terminals' pane header does the same on its close button.
#[gpui::test]
fn terminals_pane_asks_before_closing_a_pane_that_runs_something(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let (window, mut visual, calls) = mount_with(
        cx,
        &staged,
        "mods/terminals-pane",
        with_shell_pane(fixture(true, props(), updates("current", "current"))),
        None,
        |m| m,
    );
    click(&mut visual, "th-x-p1");
    assert!(ran(&calls, "pane.close"), "a shell-only pane closes at once");
    finish(cx, window, visual);

    let (window, mut visual, calls) = mount_with(
        cx,
        &staged,
        "mods/terminals-pane",
        fixture(true, props(), updates("current", "current")),
        None,
        |m| m,
    );
    click(&mut visual, "th-x-p1");
    assert!(!ran(&calls, "pane.close"), "a running pane is not closed at once");
    let ask = confirm_for(&calls, "pane.close").expect("the running pane asks first");
    assert_eq!(ask.pointer("/props/title"), Some(&json!("Close terminal?")));
    assert_eq!(ask.pointer("/props/detail"), Some(&json!("Build is still running.")));
    finish(cx, window, visual);
}

/// Ctrl+D in the Go-to picker closes a plain shell tab at once and asks before
/// closing one that runs an agent.
#[gpui::test]
fn picker_asks_before_closing_a_tab_that_runs_something(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let (window, mut visual, calls) = mount_with(
        cx,
        &staged,
        "mods/picker",
        with_shell_pane(fixture(true, props(), updates("current", "current"))),
        Some("this.handle.focus();"),
        |m| m,
    );
    press_key(&mut visual, "ctrl-d");
    assert!(ran(&calls, "tab.close"), "a shell-only tab closes at once");
    finish(cx, window, visual);

    let (window, mut visual, calls) = mount_with(
        cx,
        &staged,
        "mods/picker",
        fixture(true, props(), updates("current", "current")),
        Some("this.handle.focus();"),
        |m| m,
    );
    press_key(&mut visual, "ctrl-d");
    assert!(!ran(&calls, "tab.close"), "a running tab is not closed at once");
    let ask = confirm_for(&calls, "tab.close").expect("the running tab asks first");
    assert_eq!(ask.pointer("/props/title"), Some(&json!("Close terminal?")));
    assert_eq!(ask.pointer("/props/detail"), Some(&json!("Build is still running.")));
    finish(cx, window, visual);
}

/// Files does not poll while the panel is hidden, and refreshes as soon as it
/// is shown again.
#[gpui::test]
fn files_pauses_hidden_and_refreshes_when_shown(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let live = Rc::new(RefCell::new(main_layout(false)));
    let host = live.clone();
    let (window, mut visual, calls) = mount_with(
        cx,
        &staged,
        "mods/files",
        fixture(true, props(), updates("current", "current")),
        Some("this.refresh = (cx) => run('space.next'); cx.timer.every(100, (cx) => cx.notify());"),
        move |m| m.function("layout", move |_| Ok(to_host(&host.borrow()))),
    );
    visual.executor().advance_clock(Duration::from_secs(10));
    draw(&mut visual);
    assert_eq!(run_count(&calls, "space.next"), 0, "no tick while hidden");
    *live.borrow_mut() = main_layout(true);
    visual.executor().advance_clock(Duration::from_millis(100));
    draw(&mut visual);
    assert_eq!(run_count(&calls, "space.next"), 1, "one refresh when it appears");
    finish(cx, window, visual);
}

/// Changes follows the same rule.
#[gpui::test]
fn changes_pauses_hidden_and_refreshes_when_shown(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let live = Rc::new(RefCell::new(main_layout(false)));
    let host = live.clone();
    let (window, mut visual, calls) = mount_with(
        cx,
        &staged,
        "mods/changes",
        fixture(true, props(), updates("current", "current")),
        Some("this.refresh = (cx) => run('space.next'); cx.timer.every(100, (cx) => cx.notify());"),
        move |m| m.function("layout", move |_| Ok(to_host(&host.borrow()))),
    );
    visual.executor().advance_clock(Duration::from_secs(10));
    draw(&mut visual);
    assert_eq!(run_count(&calls, "space.next"), 0, "no tick while hidden");
    *live.borrow_mut() = main_layout_surface(true, "changes");
    visual.executor().advance_clock(Duration::from_millis(100));
    draw(&mut visual);
    assert_eq!(run_count(&calls, "space.next"), 1, "one refresh when it appears");
    finish(cx, window, visual);
}

/// Spaces checks when what it lists changes, not four times a second.
#[gpui::test]
fn spaces_checks_on_change_not_every_quarter_second(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let (window, mut visual, calls) = mount_with(
        cx,
        &staged,
        "mods/spaces",
        fixture(true, props(), updates("current", "current")),
        Some("this.checkNew = (cx) => run('space.next');"),
        |m| m,
    );
    visual.executor().advance_clock(Duration::from_secs(1));
    draw(&mut visual);
    assert_eq!(
        run_count(&calls, "space.next"),
        1,
        "one check for the first draw, no 250 ms sweep: {:?}",
        calls.borrow().effects
    );
    finish(cx, window, visual);
}

/// Spaces' slow git fallback skips while the window is not active.
#[gpui::test]
fn spaces_slow_check_skips_an_inactive_window(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let values = fixture(true, props(), updates("current", "current"));
    let base = values["window"].clone();
    let active = Rc::new(Cell::new(false));
    let awake = active.clone();
    let (window, mut visual, calls) = mount_with(
        cx,
        &staged,
        "mods/spaces",
        values,
        Some("this.checkNew = (cx) => run('space.next');"),
        move |m| {
            let base = base.clone();
            let awake = awake.clone();
            m.function("window", move |_| {
                let mut window = base.clone();
                window["active"] = json!(awake.get());
                Ok(to_host(&window))
            })
        },
    );
    visual.executor().advance_clock(Duration::from_secs(3));
    draw(&mut visual);
    assert_eq!(run_count(&calls, "space.next"), 1, "the slow check waits while asleep");
    active.set(true);
    visual.executor().advance_clock(Duration::from_secs(3));
    draw(&mut visual);
    assert_eq!(run_count(&calls, "space.next"), 2, "it runs once the window is active");
    finish(cx, window, visual);
}

/// Running and Queued rows always build their actions, dimmed until the row is
/// touched, so the keyboard can reach them.
#[gpui::test]
fn board_always_draws_running_and_queued_actions_dimmed(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut values = fixture(true, props(), updates("current", "current"));
    let dim = values["theme"]["dim"].as_str().unwrap().to_string();
    values.insert(
        "statusJson".into(),
        json!({
            "tasks": {
                "review": [],
                "running": [{"name": "run1", "repo": "repo", "worker": "opencode", "by": "manager", "seconds": 30, "cost_usd": 0.01}],
                "queued": [{"name": "wait1", "repo": "repo", "worker": "opencode", "by": "manager"}]
            },
            "claude": null
        }),
    );
    let tree = describe_settled(cx, &staged, "mods/board", values, None).unwrap();
    for id in [
        "review-running:run1",
        "cancel-running:run1",
        "watch-running:run1",
        "review-queued:wait1",
        "cancel-queued:wait1",
    ] {
        let line = tree
            .lines()
            .find(|line| line.contains(&format!(":id[Str(\"{id}\")]")))
            .unwrap_or_else(|| panic!("{id} is drawn: {tree}"));
        assert!(line.contains(&dim), "{id} is dimmed at rest: {line}");
    }
    assert!(
        !tree.contains(r#":id[Str("watch-queued:wait1")]"#),
        "a queued task has nothing to watch: {tree}"
    );
}

fn wrapper_before(staged: &Staged, entry: &str, before: &str) -> PathBuf {
    std::fs::write(
        staged.0.join("main.js"),
        format!(
            "import Real from './{entry}/main.js'; export default class Scenario extends Real {{ init(props, cx) {{ {before} super.init(props, cx); }} }}"
        ),
    )
    .unwrap();
    staged.0.clone()
}

/// A corrupt or wrongly shaped saved value falls back to the default instead
/// of breaking the mod in init.
#[gpui::test]
fn corrupt_saved_values_fall_back_to_defaults(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    for (entry, before) in [
        (
            "mods/agents",
            "localStorage.setItem('changed', '{'); localStorage.setItem('pins', '7');",
        ),
        (
            "mods/spaces",
            "localStorage.setItem('machines.reconnecting', '[]'); localStorage.setItem('machines.folded', '{}');",
        ),
        (
            "mods/topbar",
            "localStorage.setItem('panels', 'null'); localStorage.setItem('panel-last', '[]');",
        ),
        ("mods/tabs", "localStorage.setItem('panels', '\"oops\"');"),
    ] {
        wrapper_before(&staged, entry, before);
        let tree = describe_with_values(
            cx,
            &staged,
            ".",
            fixture(true, props(), updates("current", "current")),
            None,
        )
        .unwrap_or_else(|e| panic!("{entry} with corrupt storage: {e:#}"));
        assert!(!tree.is_empty(), "{entry} drew nothing");
    }
}

/// Confirm takes the keyboard late, so an Enter already on its way cannot
/// confirm the dialog.
#[gpui::test]
fn confirm_waits_before_taking_a_stray_enter(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let (window, mut visual, calls) = mount_with(
        cx,
        &staged,
        "mods/confirm",
        fixture(true, props(), updates("current", "current")),
        None,
        |m| m,
    );
    press_key(&mut visual, "enter");
    assert!(!ran(&calls, "space.rename"), "Enter before the wait does nothing");
    visual.executor().advance_clock(Duration::from_millis(650));
    visual.run_until_parked();
    draw(&mut visual);
    press_key(&mut visual, "enter");
    assert!(ran(&calls, "space.rename"), "Enter after the wait confirms");
    finish(cx, window, visual);
}

/// Spaces' "+" says so when the folder picker fails.
#[gpui::test]
fn spaces_reports_a_failed_folder_picker(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let (window, mut visual, calls) = mount_with(
        cx,
        &staged,
        "mods/spaces",
        fixture(true, props(), updates("current", "current")),
        None,
        |m| {
            m.async_function("pickFolder", |_| {
                Ok(async { Err(HostError::new("no picker")) })
            })
        },
    );
    click(&mut visual, "space-new");
    let said = calls.borrow().effects.iter().any(|(name, args)| {
        name == "notify"
            && args.first().and_then(|n| n.get("title")) == Some(&json!("Couldn't open that folder"))
    });
    assert!(said, "the failure is said in a toast: {:?}", calls.borrow().effects);
    finish(cx, window, visual);
}

/// A picked folder that differs only in its trailing separator and case is the
/// space already open, not a second one.
#[gpui::test]
fn workspace_chrome_opens_a_folder_that_is_already_a_space(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let folder = json!("/fixture/project/");
    let (window, mut visual, calls) = mount_with(
        cx,
        &staged,
        "mods/workspace-chrome",
        fixture(true, props(), updates("current", "current")),
        Some("this.spaceOpen = true;"),
        move |m| {
            let folder = folder.clone();
            m.async_function("pickFolder", move |_| {
                let folder = folder.clone();
                Ok(async move { Ok(to_host(&folder)) })
            })
        },
    );
    click(&mut visual, "space-current");
    let selected = calls.borrow().effects.iter().any(|(name, args)| {
        name == "run" && args.first() == Some(&json!("space.select")) && args.get(1) == Some(&json!({"id": "s1"}))
    });
    assert!(selected, "the existing space is selected: {:?}", calls.borrow().effects);
    assert!(!ran(&calls, "space.new"), "no second space is made");
    finish(cx, window, visual);
}

/// The git worktree removal runs before the space closes; a failure leaves the
/// space open, and a space already gone is simply not closed.
fn worktree_host(
    calls: Rc<RefCell<Calls>>,
    removed: bool,
) -> impl FnOnce(HostModule) -> HostModule {
    move |m| {
        m.async_function("exec", move |a| {
            let first = a.string(0).unwrap_or_default().to_owned();
            let args = a.get(1).map(from_host).unwrap_or(Value::Null);
            let argv: Vec<String> = args
                .as_array()
                .map(|list| list.iter().filter_map(Value::as_str).map(str::to_owned).collect())
                .unwrap_or_default();
            let joined = argv.join("\u{1}");
            let reply = if first != "git" {
                json!({"code": 1, "stdout": "", "stderr": "not git"})
            } else if joined == "rev-parse\u{1}--show-toplevel" {
                json!({"code": 0, "stdout": "/fixture/wt\n", "stderr": ""})
            } else if joined == "rev-parse\u{1}--path-format=absolute\u{1}--git-common-dir" {
                json!({"code": 0, "stdout": "/fixture/repo/.git\n", "stderr": ""})
            } else if joined == "rev-parse\u{1}--absolute-git-dir" {
                json!({"code": 0, "stdout": "/fixture/wt/.git\n", "stderr": ""})
            } else if joined == "--no-optional-locks\u{1}-c\u{1}core.fsmonitor=false\u{1}status\u{1}--porcelain" {
                json!({"code": 0, "stdout": "", "stderr": ""})
            } else if argv.first().map(String::as_str) == Some("--git-dir") {
                json!({"code": 1, "stdout": "", "stderr": "no submodules"})
            } else if argv.first().map(String::as_str) == Some("worktree") && argv.get(1).map(String::as_str) == Some("remove") {
                calls.borrow_mut().effects.push(("exec".into(), vec![json!("worktree remove")]));
                if removed {
                    json!({"code": 0, "stdout": "", "stderr": ""})
                } else {
                    json!({"code": 1, "stdout": "", "stderr": "busy"})
                }
            } else {
                json!({"code": 1, "stdout": "", "stderr": "unexpected"})
            };
            Ok(std::future::ready(Ok(to_host(&reply))))
        })
    }
}

fn remove_values(space: &str) -> Value {
    let mut p = props();
    p["action"] = json!("remove");
    p["space"] = json!(space);
    p["folder"] = json!("/fixture/wt");
    p
}

/// A successful removal closes the space after git lets go of the folder.
#[gpui::test]
fn worktree_removal_closes_the_space_after_git_succeeds(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let calls = Rc::new(RefCell::new(Calls::default()));
    let (window, mut visual, calls) = mount_with_calls(
        cx,
        &staged,
        "mods/worktree",
        fixture(true, remove_values("s1"), updates("current", "current")),
        None,
        calls.clone(),
        worktree_host(calls.clone(), true),
    );
    press_key(&mut visual, "enter");
    visual.run_until_parked();
    let effects = calls.borrow().effects.clone();
    let git = effects.iter().position(|(name, args)| name == "exec" && args.first() == Some(&json!("worktree remove")));
    let close = effects.iter().position(|(name, args)| {
        name == "run" && args.first() == Some(&json!("space.close")) && args.get(1) == Some(&json!({"id": "s1", "confirmed": true}))
    });
    assert!(git.is_some(), "the worktree was removed: {effects:?}");
    assert!(close.is_some(), "the space was closed: {effects:?}");
    assert!(git < close, "removal comes first: {effects:?}");
    finish(cx, window, visual);
}

/// A failed removal leaves the space open.
#[gpui::test]
fn worktree_removal_leaves_the_space_when_git_fails(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let calls = Rc::new(RefCell::new(Calls::default()));
    let (window, mut visual, calls) = mount_with_calls(
        cx,
        &staged,
        "mods/worktree",
        fixture(true, remove_values("s1"), updates("current", "current")),
        None,
        calls.clone(),
        worktree_host(calls.clone(), false),
    );
    press_key(&mut visual, "enter");
    visual.run_until_parked();
    assert!(!ran(&calls, "space.close"), "the space stays open: {:?}", calls.borrow().effects);
    finish(cx, window, visual);
}

/// A worktree whose space is already gone removes without trying to close it.
#[gpui::test]
fn worktree_removal_skips_a_space_that_is_gone(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let calls = Rc::new(RefCell::new(Calls::default()));
    let (window, mut visual, calls) = mount_with_calls(
        cx,
        &staged,
        "mods/worktree",
        fixture(true, remove_values("missing"), updates("current", "current")),
        None,
        calls.clone(),
        worktree_host(calls.clone(), true),
    );
    press_key(&mut visual, "enter");
    visual.run_until_parked();
    assert!(!ran(&calls, "space.close"), "no close is sent: {:?}", calls.borrow().effects);
    finish(cx, window, visual);
}

fn lead_markdown(calls: &Rc<RefCell<Calls>>) -> String {
    calls
        .borrow()
        .effects
        .iter()
        .filter(|(name, _)| name == "openOverlay")
        .find_map(|(_, args)| args.first()?.pointer("/props/markdown")?.as_str().map(str::to_owned))
        .unwrap_or_else(|| panic!("the hand-off drew: {:?}", calls.borrow().effects))
}

/// The hand-off names a used-up quota only when the status says one is out.
#[gpui::test]
fn lead_handoff_states_the_quota_only_when_it_is_out(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let (window, mut visual, calls) = mount_with(
        cx,
        &staged,
        "mods/lead",
        fixture(true, props(), updates("current", "current")),
        Some("this.flip(cx);"),
        |m| m,
    );
    let quiet = lead_markdown(&calls);
    assert!(
        quiet.starts_with("The Owner handed Manager the lead."),
        "the hand-off begins plainly: {quiet}"
    );
    assert!(!quiet.contains("quota is out"), "no false reason: {quiet}");
    finish(cx, window, visual);

    let mut values = fixture(true, props(), updates("current", "current"));
    values.insert(
        "statusJson".into(),
        json!({
            "tasks": {"review": [], "running": [], "queued": []},
            "claude": {
                "five_hour_used_percentage": 100.0, "five_hour_resets_at": 4102444800u64,
                "seven_day_used_percentage": 12.0, "seven_day_resets_at": 4102444800u64
            }
        }),
    );
    let (window, mut visual, calls) = mount_with(
        cx,
        &staged,
        "mods/lead",
        values,
        Some("this.flip(cx);"),
        |m| m,
    );
    let out = lead_markdown(&calls);
    assert!(
        out.starts_with("The Owner handed Manager the lead. Claude's quota is out."),
        "an out window is named: {out}"
    );
    finish(cx, window, visual);
}
