//! Render tests for Settings › Agents (the UX audit of 2026-11-11): see the parent module for the helpers.
#![allow(unused_imports)]

use super::*;

// Long enough to overflow a row: the core's own sentence for an out worker.
const LONG_SENTENCE: &str = "opencode Go is out until its monthly allowance is renewed on the account page, and the core will try it again at the time it names.";

fn out_worker(key: &str, label: &str, cli: &str, route: &str, window: &str, sentence: &str, until: u64) -> Value {
    let mut worker = agent_worker(key, label, cli, route, "out", window, until);
    worker["sentence"] = json!(sentence);
    worker
}

// Three days ahead, in milliseconds: a time that is neither today nor past.
fn three_days_ahead_ms() -> u64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    now + 3 * 86_400_000
}

#[gpui::test]
fn agent_out_worker_keeps_its_name_model_and_buttons(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let workers = agent_workers(
        json!(["cc", "go"]),
        json!([
            agent_worker(
                "cc",
                "Command Code",
                "command_code",
                "commandcode/deepseek/deepseek-v4.1-flash#max",
                "ready",
                "",
                0
            ),
            out_worker(
                "go",
                "opencode Go",
                "opencode",
                "opencode-go/deepseek-v4.1-flash#max",
                "",
                LONG_SENTENCE,
                three_days_ahead_ms()
            )
        ]),
    );
    let status = json!({"tasks": {"review": [], "running": [], "queued": []}, "claude": null});
    let tree = describe_settled(
        cx,
        &staged,
        "frame/settings",
        agents_values(agent_catalog(), workers, status),
        None,
    )
    .unwrap();
    let row = tree
        .find("st-worker-go")
        .unwrap_or_else(|| panic!("go row missing: {tree}"));
    let rest = &tree[row..];
    for (what, needle) in [
        ("the name", "text \"opencode Go\""),
        ("the model's name, not its id", "text \"DeepSeek V4.1 Flash\""),
        ("the raw id, as the tooltip", "tooltip[Str(\"opencode-go/deepseek-v4.1-flash\")]"),
        ("the up button", "st-worker-up-go"),
        ("the down button", "st-worker-down-go"),
        ("the words for being out", "Out · back "),
    ] {
        assert!(rest.contains(needle), "{what} is drawn in the go row: {tree}");
    }
    assert!(
        !tree.contains(&format!("text \"{LONG_SENTENCE}\"")),
        "the core's sentence is the tooltip, not the line: {tree}"
    );
    assert!(!tree.contains("text \"Out · back Out\""), "{tree}");
}

#[gpui::test]
fn agent_built_in_workers_have_no_remove_button_and_added_ones_do(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let workers = agent_workers(
        json!(["cc", "go", "haiku", "mine"]),
        json!([
            agent_worker(
                "cc",
                "Command Code",
                "command_code",
                "commandcode/deepseek/deepseek-v4.1-flash#max",
                "ready",
                "",
                0
            ),
            agent_worker(
                "go",
                "opencode Go",
                "opencode",
                "opencode-go/deepseek-v4.1-flash#max",
                "ready",
                "",
                0
            ),
            agent_worker("haiku", "Claude Haiku", "claude", "claude-haiku-5-5#high", "ready", "", 0),
            agent_worker("mine", "My worker", "opencode", "opencode/x#max", "ready", "", 0)
        ]),
    );
    let status = json!({"tasks": {"review": [], "running": [], "queued": []}, "claude": null});
    let tree = describe_settled(
        cx,
        &staged,
        "frame/settings",
        agents_values(agent_catalog(), workers, status),
        None,
    )
    .unwrap();
    assert!(tree.contains("st-worker-remove-mine"), "an added worker can be removed: {tree}");
    for key in ["cc", "go", "haiku"] {
        assert!(
            !tree.contains(&format!("st-worker-remove-{key}")),
            "the core refuses to remove {key}, so the page offers no ×: {tree}"
        );
    }
    let row = tree
        .find("st-worker-haiku")
        .unwrap_or_else(|| panic!("haiku row missing: {tree}"));
    let up = tree
        .find("st-worker-up-haiku")
        .unwrap_or_else(|| panic!("haiku up button missing: {tree}"));
    assert!(tree[row..up].contains("claude.svg"), "Claude's worker row has Claude's icon: {tree}");
    assert!(tree[row..].contains("Claude Haiku 5.5"), "the id reads as a name: {tree}");
}

/// Each worker row's arrows and × are named, tab stops with the frame's ring.
#[gpui::test]
fn agent_worker_buttons_have_names_and_tab_stops(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let workers = agent_workers(
        json!(["cc", "mine"]),
        json!([
            agent_worker("cc", "Command Code", "command_code", "commandcode/x#max", "ready", "", 0),
            agent_worker("mine", "My worker", "opencode", "opencode/x#max", "ready", "", 0)
        ]),
    );
    let status = json!({"tasks": {"review": [], "running": [], "queued": []}, "claude": null});
    let tree = describe_settled(
        cx,
        &staged,
        "frame/settings",
        agents_values(agent_catalog(), workers, status),
        None,
    )
    .unwrap();
    for (id, label) in [
        ("st-worker-up-mine", "Move up"),
        ("st-worker-down-mine", "Move down"),
        ("st-worker-remove-mine", "Remove"),
    ] {
        let line = tree
            .lines()
            .find(|line| line.contains(&format!(":id[Str(\"{id}\")]")))
            .unwrap_or_else(|| panic!("no {id}: {tree}"));
        assert!(line.contains(&format!(":tooltip[Str(\"{label}\")]")), "{id}: {line}");
        assert!(
            line.contains(&format!(":accessibility_label[Str(\"{label}\")]")),
            "{id}: {line}"
        );
        assert!(line.contains(":tab_stop"), "{id} is a tab stop: {line}");
        assert!(line.contains(":focus_ring"), "{id} has the ring: {line}");
    }
}

/// × asks first: one press only says "Remove?", and a second press within it removes the worker.
#[gpui::test]
fn agent_remove_asks_before_removing(cx: &mut TestAppContext) {
    let workers = || {
        agent_workers(
            json!(["cc", "mine"]),
            json!([
                agent_worker("cc", "Command Code", "command_code", "commandcode/x#max", "ready", "", 0),
                agent_worker("mine", "My worker", "opencode", "opencode/x#max", "ready", "", 0)
            ]),
        )
    };
    let remove = vec![json!("workers"), json!("--remove"), json!("mine")];
    let once = agents_kom_after_clicks(
        cx,
        agent_catalog(),
        workers(),
        "",
        &["st-worker-remove-mine"],
    );
    assert!(!once.contains(&remove), "the first press only asks: {once:?}");
    let twice = agents_kom_after_clicks(
        cx,
        agent_catalog(),
        workers(),
        "",
        &["st-worker-remove-mine", "st-worker-remove-mine"],
    );
    assert!(twice.contains(&remove), "the second press removes: {twice:?}");
}

/// A long lead folder is clipped in the middle, so the folder's own name stays in the row.
#[gpui::test]
fn agent_folder_keeps_the_folders_name_when_the_path_is_long(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let long = r"C:\Users\Ada\AppData\Local\kompitum\projects\clients\acme-corp-website";
    let workers = agent_workers(
        json!(["cc"]),
        json!([agent_worker("cc", "Command Code", "command_code", "commandcode/x#max", "ready", "", 0)]),
    );
    let status = json!({"tasks": {"review": [], "running": [], "queued": []}, "claude": null});
    let mut values = agents_values(agent_catalog(), workers, status);
    values.insert("person".into(), json!({"lead": {"program": "claude", "folder": long}}));
    let tree = describe_settled(cx, &staged, "frame/settings", values, None).unwrap();
    assert!(tree.contains(r"C:\\Users\\…\\clients\\acme-corp-website"), "{tree}");
    assert!(!tree.contains(r"C:\\Users\\Ada\\AppData"), "the whole path is gone: {tree}");
}

/// The Lead section is drawn as soon as the catalog is read: a slow `kom workers`
/// only leaves the Workers section saying it is reading.
#[gpui::test]
fn a_slow_workers_read_leaves_the_lead_drawn(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let workers = agent_workers(
        json!(["cc"]),
        json!([agent_worker("cc", "Command Code", "command_code", "commandcode/x#max", "ready", "", 0)]),
    );
    let status = json!({"tasks": {"review": [], "running": [], "queued": []}, "claude": null});
    let values = agents_values(agent_catalog(), workers, status);
    let calls = Rc::new(RefCell::new(Calls::default()));
    let (tx, rx) = async_channel::bounded(1);
    calls.borrow_mut().workers_gate = Some(rx);
    let policy = Policy::new()
        .with_storage_path(staged.0.join("store-slow-workers.json"))
        .with_capabilities(Capabilities::new().storage(true).read_roots([staged.0.clone()]))
        .with_host_module(module(values, calls.clone()))
        .unwrap();
    gpui_shell::policy::set_default(policy);
    let runtime = ShellRuntime::new_isolated().unwrap();
    let root = staged.0.join("frame/settings");
    let window = cx.add_window(|window, cx| Mounted(runtime.try_load(&root, window, cx).unwrap()));
    let mut visual = VisualTestContext::from_window(*window.deref(), cx);
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear(cx));
    visual.run_until_parked();
    assert!(
        visual.debug_bounds("st-lead-cli").is_some(),
        "the Lead section is drawn before kom workers answers"
    );
    assert!(
        visual.debug_bounds("st-worker-cc").is_none(),
        "the workers wait for kom workers"
    );
    tx.send_blocking(()).unwrap();
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear(cx));
    visual.run_until_parked();
    assert!(
        visual.debug_bounds("st-worker-cc").is_some(),
        "the workers draw once kom workers answers"
    );
    drop(visual);
    cx.run_until_parked();
    window.update(cx, |_, window, _| window.remove_window()).ok();
    cx.run_until_parked();
}

#[gpui::test]
fn agent_worker_action_error_is_drawn_under_the_workers_heading(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let workers = agent_workers(
        json!(["cc", "go"]),
        json!([
            agent_worker("cc", "Command Code", "command_code", "commandcode/deepseek/deepseek-v4.1-flash#max", "ready", "", 0),
            agent_worker("go", "opencode Go", "opencode", "opencode-go/deepseek-v4.1-flash#max", "ready", "", 0)
        ]),
    );
    let status = json!({"tasks": {"review": [], "running": [], "queued": []}, "claude": null});
    let tree = describe_settled(
        cx,
        &staged,
        "frame/settings",
        agents_values(agent_catalog(), workers, status),
        Some("this.agentPage.workerError = 'the core is busy with a task';"),
    )
    .unwrap();
    let heading = tree
        .find("Kompitum tries them from the top.")
        .unwrap_or_else(|| panic!("the Workers heading is drawn: {tree}"));
    let error = tree
        .find("the core is busy with a task")
        .unwrap_or_else(|| panic!("the worker error is drawn: {tree}"));
    assert!(error > heading, "the error sits under the heading, where the Owner is looking: {tree}");
}

#[gpui::test]
fn agent_up_arrow_sends_the_whole_order_as_one_argument(cx: &mut TestAppContext) {
    let workers = agent_workers(
        json!(["cc", "go"]),
        json!([
            agent_worker("cc", "Command Code", "command_code", "commandcode/deepseek/deepseek-v4.1-flash#max", "ready", "", 0),
            agent_worker("go", "opencode Go", "opencode", "opencode-go/deepseek-v4.1-flash#max", "ready", "", 0)
        ]),
    );
    let calls = agents_kom_after_clicks(cx, agent_catalog(), workers, "", &["st-worker-up-go"]);
    assert_eq!(
        calls,
        vec![vec![json!("workers"), json!("--order"), json!("go,cc")]],
        "moving go up sends the new order as one comma list"
    );
}

// A drag from one worker's row and a release on another: the page moves the worker once, as the order call.
#[gpui::test]
fn agent_drag_release_on_another_row_moves_the_worker_once(cx: &mut TestAppContext) {
    use gpui::{Modifiers, MouseButton, point, px, ScrollDelta, ScrollWheelEvent, TouchPhase};
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let workers = agent_workers(
        json!(["cc", "go"]),
        json!([
            agent_worker("cc", "Command Code", "command_code", "commandcode/deepseek/deepseek-v4.1-flash#max", "ready", "", 0),
            agent_worker("go", "opencode Go", "opencode", "opencode-go/deepseek-v4.1-flash#max", "ready", "", 0)
        ]),
    );
    let status = json!({"tasks": {"review": [], "running": [], "queued": []}, "claude": null});
    let mut values = agents_values(agent_catalog(), workers, status);
    values.insert("person".into(), json!({"lead": {"program": "claude"}}));
    let calls = Rc::new(RefCell::new(Calls::default()));
    let policy = Policy::new()
        .with_storage_path(staged.0.join("store-drag-worker.json"))
        .with_capabilities(Capabilities::new().storage(true).read_roots([staged.0.clone()]))
        .with_host_module(module(values, calls.clone()))
        .unwrap();
    gpui_shell::policy::set_default(policy);
    let runtime = ShellRuntime::new_isolated().unwrap();
    let root = staged.wrapper("frame/settings", "");
    let window = cx.add_window(|window, cx| Mounted(runtime.try_load(&root, window, cx).unwrap()));
    let mut visual = VisualTestContext::from_window(*window.deref(), cx);
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear(cx));
    visual.run_until_parked();
    // Scroll the Settings body until the go row sits in view, as the click helper does.
    for _ in 0..24 {
        let Some(bounds) = visual.debug_bounds("st-worker-go") else { break };
        let y = bounds.center().y;
        if y >= px(80.) && y <= px(580.) {
            break;
        }
        let Some(body) = visual.debug_bounds("st-body") else { break };
        let delta_y = if y > px(580.) { px(-200.) } else { px(200.) };
        visual.simulate_event(ScrollWheelEvent {
            position: body.center(),
            delta: ScrollDelta::Pixels(point(px(0.), delta_y)),
            modifiers: Modifiers::default(),
            touch_phase: TouchPhase::Moved,
        });
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        visual.run_until_parked();
    }
    let from = visual.debug_bounds("st-worker-go").expect("go row is laid out").center();
    let to = visual.debug_bounds("st-worker-cc").expect("cc row is laid out").center();
    visual.simulate_mouse_move(from, None, Modifiers::default());
    visual.simulate_mouse_down(from, MouseButton::Left, Modifiers::default());
    visual.simulate_mouse_move(to, Some(MouseButton::Left), Modifiers::default());
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear(cx));
    visual.run_until_parked();
    visual.simulate_mouse_up(to, MouseButton::Left, Modifiers::default());
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear(cx));
    visual.run_until_parked();
    drop(visual);
    cx.run_until_parked();
    window.update(cx, |_, window, _| window.remove_window()).ok();
    cx.run_until_parked();
    let calls = calls.borrow();
    assert!(calls.unexpected.is_empty(), "{:?}", calls.unexpected);
    let argvs: Vec<Vec<Value>> = calls
        .effects
        .iter()
        .filter(|(name, _)| name == "kom")
        .filter_map(|(_, args)| args.first()?.as_array().cloned())
        .collect();
    assert_eq!(
        argvs,
        vec![vec![json!("workers"), json!("--order"), json!("go,cc")]],
        "dragging go onto cc moves it once"
    );
}
