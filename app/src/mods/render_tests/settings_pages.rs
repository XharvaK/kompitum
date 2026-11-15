//! Render tests for the rework (audit/REWORK-2026-11-09.md): see the parent module for the helpers.
#![allow(unused_imports)]

use super::*;

#[gpui::test]
fn welcome_lives_on_about_not_general(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    for (section, shown) in [("general", false), ("about", true)] {
        let mut p = props();
        p["section"] = json!(section);
        let tree = describe_settled(
            cx,
            &staged,
            "frame/settings",
            fixture(true, p, updates("current", "current")),
            None,
        )
        .unwrap();
        assert_eq!(
            tree.contains("st-general-welcome"),
            shown,
            "{section}: {tree}"
        );
        assert_eq!(tree.contains("Show again"), shown, "{section}: {tree}");
    }
}

#[gpui::test]
fn keyboard_is_a_macos_page(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    for (platform, shown) in [("linux", false), ("windows", false), ("macos", true)] {
        let mut p = props();
        p["section"] = json!("keyboard");
        let mut values = fixture(true, p, updates("current", "current"));
        values.insert("window".into(), json!({"platform": platform}));
        values.insert("person".into(), json!({}));
        let tree = describe_settled(cx, &staged, "frame/settings", values, None).unwrap();
        assert_eq!(tree.contains("st-s-keyboard"), shown, "{platform}: {tree}");
        if !shown {
            assert!(
                !tree.contains("Option-key settings are for macOS."),
                "{platform}: keyboard opens Agents, not Keyboard: {tree}"
            );
            assert!(
                !tree.contains("st-option-"),
                "{platform}: no Option rows: {tree}"
            );
        }
    }
    let mut p = props();
    p["section"] = json!("general");
    let mut values = fixture(true, p, updates("current", "current"));
    values.insert("window".into(), json!({"platform": "linux"}));
    let tree = describe_settled(
        cx,
        &staged,
        "frame/settings",
        values,
        Some("this.search = 'option';"),
    )
    .unwrap();
    assert!(
        tree.contains(r#"No setting matches \"option\""#),
        "the Option search entry is left out: {tree}"
    );
}

#[gpui::test]
fn agents_page_has_no_plan_pickers(cx: &mut TestAppContext) {
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
            agent_worker(
                "go",
                "opencode Go",
                "opencode",
                "opencode-go/deepseek-v4.1-flash#max",
                "ready",
                "",
                0
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
    assert!(!tree.contains("st-plan-"), "no plan pickers: {tree}");
    assert!(!tree.contains("How much of each plan is used."), "{tree}");
    assert!(!tree.contains("Pick this CLI's plan."), "{tree}");
}

fn catalog_with_usage() -> Value {
    let mut catalog = agent_catalog();
    let opencode = catalog
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|entry| entry["key"] == "opencode")
        .unwrap();
    opencode["usage"] = json!([
        "api",
        "--standalone",
        "get",
        "/api/experimental/usage/limits"
    ]);
    catalog
}

fn opencode_worker() -> Value {
    agent_workers(
        json!(["go"]),
        json!([agent_worker(
            "go",
            "opencode Go",
            "opencode",
            "opencode-go/deepseek-v4.1-flash#max",
            "ready",
            "",
            0
        )]),
    )
}

fn opencode_agents_values(usage: Value) -> BTreeMap<String, Value> {
    let status = json!({"tasks": {"review": [], "running": [], "queued": []}, "claude": null});
    let mut values = agents_values(catalog_with_usage(), opencode_worker(), status);
    values.insert("usageJson".into(), usage);
    values
}

#[gpui::test]
fn opencode_usage_reads_the_nested_report(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let values = opencode_agents_values(json!({
        "location": {"workspace": "wrk_1"},
        "data": {
            "workspace": {"id": "wrk_1"},
            "go": {
                "rolling": {"percent": 100, "limited": false, "resetsAt": 1794256628000i64},
                "weekly": {"percent": 19, "limited": false, "resetsAt": 1794256628000i64},
                "monthly": {"percent": 96, "limited": false, "resetsAt": 1794256628000i64}
            },
            "budgets": []
        }
    }));
    let theme = values["theme"].clone();
    let tree = describe_settled(cx, &staged, "frame/settings", values, None).unwrap();
    for meter in ["5h 100%", "week 19%", "month 96%"] {
        assert!(tree.contains(meter), "missing {meter:?}: {tree}");
    }
    assert!(
        !tree.contains("wrk_1"),
        "raw output stays off screen: {tree}"
    );
    let working = theme["working"].as_str().unwrap();
    let blocked = theme["blocked"].as_str().unwrap();
    let lines: Vec<&str> = tree.lines().collect();
    let at = lines
        .iter()
        .position(|line| line.contains("text \"month 96%\""))
        .unwrap_or_else(|| panic!("no month meter: {tree}"));
    assert!(
        lines[at - 1].contains(working),
        "96% is working: {}",
        lines[at - 1]
    );
    let at = lines
        .iter()
        .position(|line| line.contains("text \"5h 100%\""))
        .unwrap_or_else(|| panic!("no 5H meter: {tree}"));
    assert!(
        lines[at - 1].contains(blocked),
        "100% is blocked: {}",
        lines[at - 1]
    );
}

#[gpui::test]
fn unreadable_usage_report_says_only_that(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let values = opencode_agents_values(json!({
        "workspace": "wrk_9c1",
        "note": "raw-report-marker"
    }));
    let tree = describe_settled(cx, &staged, "frame/settings", values, None).unwrap();
    assert!(
        tree.contains("OpenCode's usage report couldn't be read."),
        "{tree}"
    );
    assert!(
        !tree.contains("wrk_9c1"),
        "raw output stays off screen: {tree}"
    );
    assert!(
        !tree.contains("raw-report-marker"),
        "raw output stays off screen: {tree}"
    );
}

fn lead_values(program: &str) -> BTreeMap<String, Value> {
    let status = json!({"tasks": {"review": [], "running": [], "queued": []}, "claude": null});
    let mut values = agents_values(
        agent_catalog(),
        agent_workers(json!([]), json!([])),
        status,
    );
    values.insert("person".into(), json!({"lead": {"program": program}}));
    values
}

#[gpui::test]
fn lead_closed_shows_only_the_chosen_cli(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe_settled(
        cx,
        &staged,
        "frame/settings",
        lead_values("opencode"),
        None,
    )
    .unwrap();
    assert!(tree.contains("st-lead-cli"), "{tree}");
    assert!(
        tree.contains("OpenCode"),
        "the chosen CLI's name is in the trigger: {tree}"
    );
    assert!(
        !tree.contains("st-lead-cli-claude"),
        "another CLI's row is absent: {tree}"
    );
    assert!(
        !tree.contains("st-lead-cli-command-code"),
        "another CLI's row is absent: {tree}"
    );
}

#[gpui::test]
fn lead_opened_lists_every_cli(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe_settled(
        cx,
        &staged,
        "frame/settings",
        lead_values("opencode"),
        Some("this.agentPage.leadCli.open = true;"),
    )
    .unwrap();
    for key in ["claude", "command-code", "opencode"] {
        assert!(
            tree.contains(&format!("st-lead-cli-{key}")),
            "missing the {key} row: {tree}"
        );
    }
}

/// A described element's line without its `:hover(…)` and `:active(…)` groups.
fn without_state_styles(line: &str) -> String {
    let mut out = line.to_string();
    for open in [":hover(", ":active("] {
        while let Some(start) = out.find(open) {
            let mut depth = 0;
            let mut end = out.len();
            for (i, c) in out[start + open.len() - 1..].char_indices() {
                match c {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            end = start + open.len() - 1 + i + 1;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            out.replace_range(start..end, "");
        }
    }
    out
}

/// An open list draws its highlight on one row, in the theme's hover colour, from the state the caller keeps.
#[gpui::test]
fn an_open_dropdown_marks_the_highlighted_row_with_the_hover_colour(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let values = lead_values("opencode");
    let hover = values["theme"]["hover"].as_str().unwrap().to_string();
    let tree = describe_settled(
        cx,
        &staged,
        "frame/settings",
        values,
        Some("this.agentPage.leadCli.open = true; this.agentPage.leadCli.hi = 1;"),
    )
    .unwrap();
    for (key, lit) in [("claude", false), ("command-code", true), ("opencode", false)] {
        let row = tree
            .lines()
            .find(|line| line.contains(&format!("st-lead-cli-{key}")))
            .unwrap_or_else(|| panic!("no {key} row: {tree}"));
        // The row's own style, without its hover and press styles (which use the hover colour on every row).
        let base = without_state_styles(row);
        assert_eq!(
            base.contains(&format!(".bg[Str(\"{hover}\")]")),
            lit,
            "{key}: {row}"
        );
    }
}

/// Mounts Settings, clicks each id, optionally hovers one, sends `keys`, and
/// returns the settings.json texts written plus whether each of `want` is laid
/// out afterwards.
fn settings_after_keys(
    cx: &mut TestAppContext,
    values: BTreeMap<String, Value>,
    setup: &str,
    clicks: &[&'static str],
    hover: Option<&'static str>,
    keys: &str,
    want: &[&'static str],
) -> (Vec<Value>, Vec<(String, bool)>) {
    use gpui::{Modifiers, MouseButton};
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let calls = Rc::new(RefCell::new(Calls::default()));
    let policy = Policy::new()
        .with_storage_path(staged.0.join("store-dropdown.json"))
        .with_capabilities(
            Capabilities::new()
                .storage(true)
                .read_roots([staged.0.clone()]),
        )
        .with_host_module(module(values, calls.clone()))
        .unwrap();
    gpui_shell::policy::set_default(policy);
    let runtime = ShellRuntime::new_isolated().unwrap();
    let root = staged.wrapper("frame/settings", setup);
    let window = cx.add_window(|window, cx| Mounted(runtime.try_load(&root, window, cx).unwrap()));
    let mut visual = VisualTestContext::from_window(*window.deref(), cx);
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear(cx));
    visual.run_until_parked();
    for id in clicks {
        let at = visual
            .debug_bounds(id)
            .unwrap_or_else(|| panic!("{id} is laid out"))
            .center();
        visual.simulate_mouse_move(at, None, Modifiers::default());
        visual.simulate_mouse_down(at, MouseButton::Left, Modifiers::default());
        visual.simulate_mouse_up(at, MouseButton::Left, Modifiers::default());
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        visual.run_until_parked();
    }
    if let Some(id) = hover {
        let at = visual
            .debug_bounds(id)
            .unwrap_or_else(|| panic!("{id} is laid out"))
            .center();
        visual.simulate_mouse_move(at, None, Modifiers::default());
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        visual.run_until_parked();
    }
    visual.simulate_keystrokes(keys);
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear(cx));
    visual.run_until_parked();
    let found = want
        .iter()
        .map(|id| ((*id).to_string(), visual.debug_bounds(id).is_some()))
        .collect();
    drop(visual);
    cx.run_until_parked();
    window.update(cx, |_, window, _| window.remove_window()).ok();
    cx.run_until_parked();
    let calls = calls.borrow();
    assert!(calls.unexpected.is_empty(), "{:?}", calls.unexpected);
    let written = calls
        .effects
        .iter()
        .filter(|(name, _)| name == "writeTextFile")
        .filter_map(|(_, args)| args.get(1).cloned())
        .collect();
    (written, found)
}

fn written_settings(writes: &[Value]) -> Value {
    let text = writes
        .last()
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("settings.json was not written: {writes:?}"));
    serde_json::from_str(text).unwrap()
}

/// Mounts Settings, clicks each id, and returns every host effect and whether each of `want` is
/// laid out afterwards.
fn settings_effects_after_clicks(
    cx: &mut TestAppContext,
    values: BTreeMap<String, Value>,
    setup: &str,
    clicks: &[&'static str],
    want: &[&'static str],
) -> (Vec<(String, Vec<Value>)>, Vec<(String, bool)>) {
    use gpui::{Modifiers, MouseButton};
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let calls = Rc::new(RefCell::new(Calls::default()));
    let policy = Policy::new()
        .with_storage_path(staged.0.join("store-settings-effects.json"))
        .with_capabilities(Capabilities::new().storage(true).read_roots([staged.0.clone()]))
        .with_host_module(module(values, calls.clone()))
        .unwrap();
    gpui_shell::policy::set_default(policy);
    let runtime = ShellRuntime::new_isolated().unwrap();
    let root = staged.wrapper("frame/settings", setup);
    let window = cx.add_window(|window, cx| Mounted(runtime.try_load(&root, window, cx).unwrap()));
    let mut visual = VisualTestContext::from_window(*window.deref(), cx);
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear(cx));
    visual.run_until_parked();
    for id in clicks {
        let at = visual
            .debug_bounds(id)
            .unwrap_or_else(|| panic!("{id} is laid out"))
            .center();
        visual.simulate_mouse_move(at, None, Modifiers::default());
        visual.simulate_mouse_down(at, MouseButton::Left, Modifiers::default());
        visual.simulate_mouse_up(at, MouseButton::Left, Modifiers::default());
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        visual.run_until_parked();
    }
    let found = want
        .iter()
        .map(|id| ((*id).to_string(), visual.debug_bounds(id).is_some()))
        .collect();
    drop(visual);
    cx.run_until_parked();
    window.update(cx, |_, window, _| window.remove_window()).ok();
    cx.run_until_parked();
    let calls = calls.borrow();
    assert!(calls.unexpected.is_empty(), "{:?}", calls.unexpected);
    (calls.effects.clone(), found)
}

/// Down moves the highlight to the next CLI, and Enter picks it.
#[gpui::test]
fn lead_dropdown_down_and_enter_pick_the_next_cli(cx: &mut TestAppContext) {
    let (writes, found) = settings_after_keys(
        cx,
        lead_values("opencode"),
        "",
        &["st-lead-cli"],
        None,
        "down enter",
        &["st-lead-cli-list", "st-lead-cli-claude"],
    );
    assert_eq!(found, vec![("st-lead-cli-list".into(), false), ("st-lead-cli-claude".into(), false)]);
    assert_eq!(written_settings(&writes)["lead"]["program"], json!("command-code"));
}

/// Escape closes the list and leaves the lead as it was.
#[gpui::test]
fn lead_dropdown_escape_closes_without_picking(cx: &mut TestAppContext) {
    let (writes, found) = settings_after_keys(
        cx,
        lead_values("opencode"),
        "",
        &["st-lead-cli"],
        None,
        "escape",
        &["st-lead-cli-list", "st-lead-cli-claude"],
    );
    assert_eq!(found, vec![("st-lead-cli-list".into(), false), ("st-lead-cli-claude".into(), false)]);
    assert!(writes.is_empty(), "nothing was picked: {writes:?}");
}

/// Hovering a row moves the highlight there, and Enter picks that row.
#[gpui::test]
fn lead_dropdown_hover_moves_the_highlight(cx: &mut TestAppContext) {
    let (writes, _) = settings_after_keys(
        cx,
        lead_values("opencode"),
        "",
        &["st-lead-cli"],
        Some("st-lead-cli-command-code"),
        "enter",
        &[],
    );
    assert_eq!(written_settings(&writes)["lead"]["program"], json!("command-code"));
}

/// With a search field, the arrow keys work while it has the focus.
#[gpui::test]
fn model_dropdown_arrows_work_while_the_search_field_has_focus(cx: &mut TestAppContext) {
    let (writes, found) = settings_after_keys(
        cx,
        lead_values("claude"),
        "",
        &["st-lead-model"],
        None,
        "down enter",
        &["st-lead-model-list"],
    );
    assert_eq!(found, vec![("st-lead-model-list".into(), false)]);
    assert_eq!(written_settings(&writes)["lead"]["model"], json!("opus"));
}

#[gpui::test]
fn agents_page_has_a_lead_folder_row(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe_settled(
        cx,
        &staged,
        "frame/settings",
        lead_values("claude"),
        None,
    )
    .unwrap();
    assert!(tree.contains("Folder"), "{tree}");
    assert!(tree.contains("st-lead-folder"), "{tree}");
    assert!(tree.contains("Not set"), "{tree}");
    assert!(
        !tree.contains("/fixture"),
        "the lead's folder never defaults to home: {tree}"
    );
}

#[gpui::test]
fn the_code_mode_switch_writes_the_lead_setting(cx: &mut TestAppContext) {
    let (writes, found) = settings_after_keys(
        cx,
        lead_values("claude"),
        "",
        &["st-code-mode"],
        None,
        "",
        &["st-code-mode"],
    );
    assert_eq!(found, vec![("st-code-mode".into(), true)]);
    assert_eq!(written_settings(&writes)["lead"]["code_mode"], json!(true));
}

fn code_mode_values(code_mode: bool, running: Value) -> BTreeMap<String, Value> {
    let mut values = lead_values("claude");
    values.insert(
        "person".into(),
        json!({"lead": {"program": "claude", "code_mode": code_mode}}),
    );
    values.insert("lead".into(), running);
    values
}

#[gpui::test]
fn the_code_mode_row_says_the_running_sessions_status(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let cases = [
        (json!("on"), Some("On in this session.")),
        (json!("off"), Some("Off in this session.")),
        (
            json!({"unavailable": "needs an interactive terminal"}),
            Some("Not available: needs an interactive terminal"),
        ),
    ];
    for (status, want) in cases {
        let tree = describe_settled(
            cx,
            &staged,
            "frame/settings",
            code_mode_values(true, json!({ "code_mode_status": status })),
            None,
        )
        .unwrap();
        assert!(tree.contains("Code Mode (experimental)"), "{tree}");
        assert!(tree.contains("st-code-mode"), "{tree}");
        assert!(tree.contains("The first time, Claude Code asks in the lead's terminal"), "{tree}");
        let want = want.expect("a status line");
        assert!(tree.contains(want), "missing {want}: {tree}");
    }
    let none = describe_settled(
        cx,
        &staged,
        "frame/settings",
        code_mode_values(true, Value::Null),
        None,
    )
    .unwrap();
    for gone in ["On in this session.", "Off in this session.", "Not available:"] {
        assert!(!none.contains(gone), "no lead runs: {gone}: {none}");
    }
    let off = describe_settled(
        cx,
        &staged,
        "frame/settings",
        code_mode_values(false, json!({ "code_mode_status": "off" })),
        None,
    )
    .unwrap();
    assert!(!off.contains("Off in this session."), "the switch is off: {off}");
    assert!(!off.contains("The first time, Claude Code asks"), "{off}");
    // Turned off while a Code Mode session runs: that session keeps it, and says so.
    let kept = describe_settled(
        cx,
        &staged,
        "frame/settings",
        code_mode_values(false, json!({ "code_mode_status": "on" })),
        None,
    )
    .unwrap();
    assert!(kept.contains("On in this session."), "the session keeps its mode: {kept}");
    assert!(!kept.contains("The first time, Claude Code asks"), "{kept}");
}

#[gpui::test]
fn font_dropdown_search_narrows_the_list(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut p = props();
    p["section"] = json!("appearance");
    let mut values = fixture(true, p, updates("current", "current"));
    values.insert("person".into(), json!({}));
    let tree = describe_settled(
        cx,
        &staged,
        "frame/settings",
        values,
        Some("this.fontMenus = { font: { open: true, query: 'sans' } };"),
    )
    .unwrap();
    assert!(tree.contains("st-f-font-Sans"), "{tree}");
    assert!(
        !tree.contains("st-f-font-Monospace"),
        "only matching fonts are drawn: {tree}"
    );
}

#[gpui::test]
fn a_typed_model_id_can_be_used(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe_settled(
        cx,
        &staged,
        "frame/settings",
        lead_values("opencode"),
        Some("this.agentPage.leadModel.open = true; this.agentPage.leadModel.query = 'my/model-9';"),
    )
    .unwrap();
    assert!(tree.contains("st-lead-model-my/model-9"), "{tree}");
    assert!(tree.contains("Use this model id"), "{tree}");
}

#[gpui::test]
fn a_model_list_that_fails_says_so(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe_settled(
        cx,
        &staged,
        "frame/settings",
        lead_values("opencode"),
        Some("this.agentPage.listModels.opencode = { loading: false, lines: [], error: 'not signed in' };"),
    )
    .unwrap();
    assert!(tree.contains("models: not signed in"), "{tree}");
}

fn add_worker_values(catalog: Value, workers: Value) -> BTreeMap<String, Value> {
    let status = json!({"tasks": {"review": [], "running": [], "queued": []}, "claude": null});
    let mut values = agents_values(catalog, workers, status);
    values.insert("person".into(), json!({"lead": {"program": "claude"}}));
    values
}

fn builtins_waiting_to_be_added() -> Value {
    agent_workers(
        json!(["cc"]),
        json!([
            agent_worker(
                "cc",
                "Command Code",
                "command_code",
                "commandcode/claude-sonnet-4-6#max",
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
            agent_worker("free", "OpenCode Free", "opencode", "opencode/free", "ready", "", 0),
            agent_worker("codex", "Codex", "codex", "codex", "ready", "", 0),
        ]),
    )
}

fn catalog_without_worker_efforts() -> Value {
    json!([
        {
            "key": "claude", "name": "Claude Code", "icon": "icons/claude.svg", "tint": false,
            "tile": "#1c1d21", "color": "#ffffff", "shell": false, "agent": true,
            "run": ["claude"], "installed": true,
            "flags": {"model": ["--model", "{model}"], "effort": ["--effort", "{effort}"]},
            "efforts": ["low", "medium", "high", "xhigh", "max"],
            "models": [
                {"id": "fable", "label": "Fable", "hint": "The most capable. Has its own weekly limit."}
            ]
        },
        {
            "key": "plain", "name": "Plain", "icon": "icons/opencode.svg", "tint": true,
            "tile": "#1c1d21", "color": "#ffffff", "shell": false, "agent": true,
            "run": ["plain"], "installed": true,
            "flags": {"model": ["-m", "{model}"]},
            "models": [{"id": "m1", "label": "Model One"}],
            "worker": "plain", "route": "{model}"
        }
    ])
}

fn catalog_without_a_worker_cli() -> Value {
    json!([{
        "key": "claude", "name": "Claude Code", "icon": "icons/claude.svg", "tint": false,
        "tile": "#1c1d21", "color": "#ffffff", "shell": false, "agent": true,
        "run": ["claude"], "installed": true,
        "flags": {"model": ["--model", "{model}"], "effort": ["--effort", "{effort}"]},
        "efforts": ["low", "medium", "high", "xhigh", "max"],
        "models": [
            {"id": "fable", "label": "Fable", "hint": "The most capable. Has its own weekly limit."}
        ]
    }])
}

/// Add a worker opens a card: built-ins not in the order, then Claude's subagents.
#[gpui::test]
fn adding_a_worker_lists_builtins_not_in_the_order_then_claude_subagents(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe_settled(
        cx,
        &staged,
        "frame/settings",
        add_worker_values(agent_catalog(), builtins_waiting_to_be_added()),
        Some("this.agentPage.addOpen = true;"),
    )
    .unwrap();
    assert!(
        tree.contains(r#":id[Str("st-add")]"#),
        "the card has id st-add: {tree}"
    );
    assert!(tree.contains("Ready to add"), "{tree}");
    for key in ["go", "free", "codex", "sonnet", "haiku"] {
        assert!(
            tree.contains(&format!("st-add-ready-{key}")),
            "missing ready row {key}: {tree}"
        );
    }
    assert!(
        !tree.contains("st-add-ready-cc"),
        "cc is already in the order: {tree}"
    );
    let go = tree.find("st-add-ready-go").expect("go row");
    let sonnet = tree.find("st-add-ready-sonnet").expect("sonnet row");
    assert!(
        go < sonnet,
        "built-ins come before Claude subagents: {tree}"
    );
    assert!(tree.contains("Free models through OpenCode."), "{tree}");
    assert!(tree.contains("Codex with its own default model."), "{tree}");
    assert!(
        tree.contains("A subagent inside the lead's Claude Code, on Sonnet."),
        "{tree}"
    );
    assert!(
        tree.contains("A subagent inside the lead's Claude Code, on Haiku."),
        "{tree}"
    );
    assert!(tree.contains("st-add-close"), "{tree}");
    assert!(
        !tree.contains(r#"text "Done""#),
        "Done is gone: {tree}"
    );
    assert!(!tree.contains("st-add-builtin-"), "old builtin chips: {tree}");
    assert!(!tree.contains("st-add-subagent-"), "old subagent chips: {tree}");
}

/// Choose your own starts on the first installed worker CLI, its first model, and max.
#[gpui::test]
fn adding_a_worker_shows_cli_model_and_effort_with_the_first_picks(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe_settled(
        cx,
        &staged,
        "frame/settings",
        add_worker_values(agent_catalog(), builtins_waiting_to_be_added()),
        Some("this.agentPage.addOpen = true;"),
    )
    .unwrap();
    assert!(tree.contains("Choose your own"), "{tree}");
    assert!(tree.contains("st-add-cli"), "{tree}");
    assert!(tree.contains("st-add-model"), "{tree}");
    assert!(tree.contains("st-add-effort"), "{tree}");
    assert!(tree.contains("st-add-go"), "{tree}");
    assert!(
        tree.contains("Command Code"),
        "the first worker CLI is picked: {tree}"
    );
    assert!(
        tree.contains("Claude Sonnet 4.6"),
        "that CLI's first model is picked: {tree}"
    );
    assert!(tree.contains("Max"), "max is picked when the CLI has it: {tree}");
    assert!(
        !tree.contains("st-add-model-command-code-"),
        "old model chips: {tree}"
    );
    assert!(
        !tree.contains("st-add-effort-command-code-"),
        "old effort chips: {tree}"
    );
    assert!(
        !tree.contains("st-add-worker-command-code") && !tree.contains("st-add-worker-opencode"),
        "old per-CLI Add worker buttons: {tree}"
    );
    let button = tree
        .lines()
        .find(|line| line.contains("st-add-go"))
        .unwrap_or_else(|| panic!("no own-worker Add button: {tree}"));
    let theme = fixture(false, props(), updates("current", "current"))["theme"].clone();
    assert!(button.contains(theme["accent_fill"].as_str().unwrap()), "{button}");
    assert!(button.contains(theme["accent_foreground"].as_str().unwrap()), "{button}");
    assert!(
        !button.contains(&format!(".bg[Str(\"{}\")]", theme["solid"].as_str().unwrap())),
        "{button}"
    );
}

/// Changing the CLI through its dropdown row resets the model to that CLI's first.
#[gpui::test]
fn adding_a_worker_resets_the_model_when_the_cli_changes(cx: &mut TestAppContext) {
    let sent = agents_kom_after_clicks(
        cx,
        agent_catalog(),
        builtins_waiting_to_be_added(),
        "this.agentPage.addOpen = true; this.agentPage.addCli = { open: true };",
        &["st-add-cli-opencode", "st-add-go"],
    );
    let add = sent
        .iter()
        .find(|args| args.get(1) == Some(&json!("--add")))
        .unwrap_or_else(|| panic!("expected workers --add, got {sent:?}"));
    assert_eq!(add.get(4), Some(&json!("opencode")), "cli: {add:?}");
    assert_eq!(
        add.get(6),
        Some(&json!("opencode-go/deepseek-v4.1-flash#max")),
        "route uses the new CLI's first model: {add:?}"
    );
    assert_eq!(
        add.get(8),
        Some(&json!("OpenCode · DeepSeek V4.1 Flash")),
        "label uses the new CLI's first model: {add:?}"
    );
}

/// A CLI with no efforts hides the reasoning dropdown.
#[gpui::test]
fn adding_a_worker_hides_effort_when_the_cli_has_none(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe_settled(
        cx,
        &staged,
        "frame/settings",
        add_worker_values(catalog_without_worker_efforts(), agent_workers(json!([]), json!([]))),
        Some("this.agentPage.addOpen = true;"),
    )
    .unwrap();
    assert!(tree.contains("st-add-cli"), "{tree}");
    assert!(tree.contains("st-add-model"), "{tree}");
    assert!(tree.contains("Plain"), "{tree}");
    assert!(tree.contains("Model One"), "{tree}");
    assert!(
        !tree.contains("st-add-effort"),
        "no reasoning dropdown: {tree}"
    );
}

/// No installed worker CLI is a dim line, not empty dropdowns.
#[gpui::test]
fn adding_a_worker_says_when_no_worker_cli_is_installed(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe_settled(
        cx,
        &staged,
        "frame/settings",
        add_worker_values(catalog_without_a_worker_cli(), agent_workers(json!([]), json!([]))),
        Some("this.agentPage.addOpen = true;"),
    )
    .unwrap();
    assert!(
        tree.contains("No other worker CLI is installed. Install Codex or OpenCode to add one."),
        "{tree}"
    );
    assert!(!tree.contains("st-add-cli"), "no CLI dropdown: {tree}");
    assert!(!tree.contains("st-add-model"), "no model dropdown: {tree}");
    assert!(!tree.contains("st-add-go"), "no Add: {tree}");
}

#[gpui::test]
fn a_failed_vault_pull_is_not_up_to_date(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut p = props();
    p["section"] = json!("vault");
    let mut values = fixture(true, p, updates("current", "current"));
    values.insert(
        "vaultJson".into(),
        json!({
            "vault": "/fixture/vault", "agent": "owner", "unpushed": 0,
            "problems": [
                "The vault couldn't be updated, so this may be behind (git: fatal: Cannot rebase onto multiple branches)."
            ],
            "notes": []
        }),
    );
    let theme = values["theme"].clone();
    let tree = describe_settled(cx, &staged, "frame/settings", values, None).unwrap();
    assert!(tree.contains("Couldn't pull \u{b7} just now"), "{tree}");
    assert!(!tree.contains("Up to date"), "{tree}");
    assert!(
        tree.contains("Another pull was running. Trying again."),
        "the plain words name the busy puller: {tree}"
    );
    assert!(tree.contains("st-vault-retry"), "Retry is kept: {tree}");

    let blocked = theme["blocked"].as_str().unwrap();
    let lines: Vec<&str> = tree.lines().collect();
    let at = lines
        .iter()
        .position(|line| line.contains("text \"Couldn't pull"))
        .unwrap_or_else(|| panic!("no Couldn't pull label: {tree}"));
    assert!(
        lines[at - 1].contains(blocked),
        "the State label is blocked: {}",
        lines[at - 1]
    );
    assert!(
        !lines[at].contains("Cannot rebase onto multiple branches"),
        "git's text is not the State label: {}",
        lines[at]
    );

    let raw = "Cannot rebase onto multiple branches";
    let mut seen = false;
    for line in tree.lines().filter(|line| line.contains(raw)) {
        seen = true;
        assert!(line.contains(":tooltip["), "git's text stays in the tooltip: {line}");
    }
    assert!(seen, "the git detail is kept as a tooltip: {tree}");
}

#[gpui::test]
fn a_note_problem_is_not_a_failed_pull(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut p = props();
    p["section"] = json!("vault");
    let mut values = fixture(true, p, updates("current", "current"));
    values.insert(
        "vaultJson".into(),
        json!({
            "vault": "/fixture/vault", "agent": "owner", "unpushed": 0,
            "problems": ["fixture: a note's mark file is damaged"],
            "notes": []
        }),
    );
    let tree = describe_settled(
        cx,
        &staged,
        "frame/settings",
        values,
        Some("this.advancedOpen = { vault: true };"),
    )
    .unwrap();
    assert!(tree.contains("Up to date"), "{tree}");
    assert!(!tree.contains("Couldn't pull"), "{tree}");
    assert!(
        tree.contains("fixture: a note's mark file is damaged"),
        "the note's own problem is still drawn: {tree}"
    );
}

/// The Settings helper maps git's causes to plain words; the raw text never shows.
#[gpui::test]
fn pull_words_name_the_cause_without_git_text(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    std::fs::write(
        staged.0.join("main.js"),
        r#"
import { View, div } from 'gpui-kit';
import { pullWords } from './frame/settings/vault.js';
export default class Probe extends View {
  render() {
    return div().children([
      div().child(`lock: ${pullWords("fatal: Unable to create '/v/.git/index.lock': File exists.")}`),
      div().child(`branches: ${pullWords('fatal: Cannot rebase onto multiple branches')}`),
      div().child(`timeout: ${pullWords('git fetch --quiet took longer than 15 s')}`),
      div().child(`other: ${pullWords("fatal: no upstream configured for branch 'main'")}`),
    ]);
  }
}
"#,
    )
    .unwrap();
    let tree = describe(
        cx,
        &staged,
        ".",
        false,
        props(),
        updates("current", "current"),
        None,
    )
    .unwrap();
    assert!(
        tree.contains("lock: Another pull was running. Trying again."),
        "{tree}"
    );
    assert!(
        tree.contains("branches: Another pull was running. Trying again."),
        "{tree}"
    );
    assert!(tree.contains("timeout: The pull took too long."), "{tree}");
    assert!(
        tree.contains("other: The vault could not be pulled."),
        "{tree}"
    );
    assert!(!tree.contains("Unable to create"), "{tree}");
    assert!(!tree.contains("Cannot rebase onto multiple branches"), "{tree}");
}

/// General is gone: its one row, the lead's folder, lives on Agents, and an old
/// `general` section still opens there.
#[gpui::test]
fn general_is_gone_and_its_section_opens_agents(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut values = lead_values("claude");
    let mut p = props();
    p["section"] = json!("general");
    values.insert("props".into(), p);
    values.insert(
        "person".into(),
        json!({"lead": {"program": "claude", "folder": "/fixture/projects"}}),
    );
    let tree = describe_settled(cx, &staged, "frame/settings", values, None).unwrap();
    assert!(!tree.contains("Home folder"), "{tree}");
    assert!(!tree.contains("Onboarding"), "{tree}");
    assert!(tree.contains("st-lead-folder"), "general opens Agents: {tree}");
}

#[gpui::test]
fn settings_list_has_connect_but_no_general_or_machines(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut p = props();
    p["section"] = json!("appearance");
    let tree = describe(
        cx,
        &staged,
        "frame/settings",
        true,
        p,
        updates("current", "current"),
        None,
    )
    .unwrap();
    assert!(tree.contains("st-s-connect"), "{tree}");
    assert!(!tree.contains("st-s-general"), "{tree}");
    assert!(!tree.contains("st-s-machines"), "{tree}");
}

/// A system with no entry shows only the Connect button, never "Not connected".
#[gpui::test]
fn connect_system_without_an_entry_has_connect_and_no_state(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let found = json!({"program": "C:\\Program Files\\Grok Bot\\grokbot.exe", "icon": null});
    let tree = describe_settled(
        cx,
        &staged,
        "frame/settings",
        connect_values(found, Value::Null),
        None,
    )
    .unwrap();
    assert!(tree.contains("st-c-connect-grokbot"), "{tree}");
    assert!(tree.contains(r#"text "Connect""#), "{tree}");
    assert!(!tree.contains("Not connected"), "{tree}");
    assert!(!tree.contains("Waiting for it to join"), "{tree}");
    assert!(!tree.contains("Joined as"), "{tree}");
}

/// An entry with only `since` says it waits for the system to join.
#[gpui::test]
fn connect_system_with_only_since_waits_for_it_to_join(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let found = json!({"program": "C:\\Program Files\\Grok Bot\\grokbot.exe", "icon": null});
    let tree = describe_settled(
        cx,
        &staged,
        "frame/settings",
        connect_values(
            found,
            json!({"connect": {"grokbot": {"since": "2026-11-11T10:00:00.000Z"}}}),
        ),
        None,
    )
    .unwrap();
    assert!(tree.contains("Waiting for it to join"), "{tree}");
    assert!(tree.contains("st-c-prompt-grokbot"), "{tree}");
    assert!(!tree.contains("st-c-connect-grokbot"), "{tree}");
    assert!(!tree.contains("Not connected"), "{tree}");
}

/// A waiting system has joined once one agent has written to the lead since its prompt was
/// copied, with or without a note titled "Joined". The fixture's note is from 09:00 UTC.
#[gpui::test]
fn connect_system_that_wrote_since_the_prompt_has_joined(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let found = json!({"program": "C:\\Program Files\\Grok Bot\\grokbot.exe", "icon": null});
    let tree = describe_settled(
        cx,
        &staged,
        "frame/settings",
        connect_values(
            found,
            json!({"connect": {"grokbot": {"since": "2026-11-11T08:00:00.000Z"}}}),
        ),
        None,
    )
    .unwrap();
    assert!(tree.contains("Joined as manager"), "{tree}");
    assert!(!tree.contains("Waiting for it to join"), "{tree}");
}

/// An entry with an agent says it joined as that agent.
#[gpui::test]
fn connect_system_with_an_agent_says_joined_as(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let found = json!({"program": "C:\\Program Files\\Grok Bot\\grokbot.exe", "icon": null});
    let tree = describe_settled(
        cx,
        &staged,
        "frame/settings",
        connect_values(
            found,
            json!({"connect": {"grokbot": {"since": "2026-11-11T10:00:00.000Z", "agent": "grok"}}}),
        ),
        None,
    )
    .unwrap();
    assert!(tree.contains("Joined as grok"), "{tree}");
    assert!(!tree.contains("Connected as"), "{tree}");
}

/// Connect opens with Machines: its heading, then This computer, then the agent systems.
#[gpui::test]
fn connect_draws_machines_first_with_this_computer_under_it(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let found = json!({"program": "C:\\Program Files\\Grok Bot\\grokbot.exe", "icon": null});
    let tree = describe_settled(
        cx,
        &staged,
        "frame/settings",
        connect_values(found, Value::Null),
        None,
    )
    .unwrap();
    let heading = tree
        .find(r#"text "Machines""#)
        .unwrap_or_else(|| panic!("no Machines heading: {tree}"));
    let this = tree
        .find(r#"text "This computer""#)
        .unwrap_or_else(|| panic!("no This computer row: {tree}"));
    let systems = tree
        .find(r#"text "Agents from other systems""#)
        .unwrap_or_else(|| panic!("no agents heading: {tree}"));
    assert!(heading < this && this < systems, "order: {tree}");
    assert!(tree.contains(r#"text "Connected""#), "{tree}");
    assert!(!tree.contains("Connected ·"), "This computer says only Connected: {tree}");
}

/// The old section name `machines` opens Connect.
#[gpui::test]
fn machines_section_name_opens_connect(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut p = props();
    p["section"] = json!("machines");
    let mut values = fixture(true, p, updates("current", "current"));
    values.insert("person".into(), Value::Null);
    let tree = describe_settled(cx, &staged, "frame/settings", values, None).unwrap();
    assert!(tree.contains(r#"text "Machines""#), "{tree}");
    assert!(tree.contains("st-c-other"), "{tree}");
}

#[gpui::test]
fn about_source_build_says_it_does_not_update_itself(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut p = props();
    p["section"] = json!("about");
    let tree = describe(
        cx,
        &staged,
        "frame/settings",
        true,
        p,
        updates("source", "current"),
        None,
    )
    .unwrap();
    assert!(
        tree.contains("This build was made from source, so it doesn't update itself."),
        "{tree}"
    );
    assert!(!tree.contains("Not checked"), "{tree}");
    assert!(tree.contains(r#"text "0.1.0""#), "the App row shows the version: {tree}");
}

#[gpui::test]
fn notifications_ask_the_lead_for_sounds(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut p = props();
    p["section"] = json!("notifications");
    let tree = describe(
        cx,
        &staged,
        "frame/settings",
        true,
        p,
        updates("current", "current"),
        None,
    )
    .unwrap();
    assert!(
        tree.contains("Want your own sounds? Ask your lead to change them."),
        "{tree}"
    );
    assert!(!tree.contains("MP3"), "{tree}");
}

/// The title bar says where the person is: Settings › the page, or Search while the field has text.
#[gpui::test]
fn settings_shows_where_you_are(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    for (section, title) in [("appearance", "Appearance"), ("vault", "Vault")] {
        let mut p = props();
        p["section"] = json!(section);
        let tree = describe(
            cx,
            &staged,
            "frame/settings",
            true,
            p,
            updates("current", "current"),
            None,
        )
        .unwrap();
        assert!(
            tree.contains(&format!("Settings › {title}")),
            "{section}: the sheet's title says where you are: {tree}"
        );
    }
    let tree = describe(
        cx,
        &staged,
        "frame/settings",
        true,
        props(),
        updates("current", "current"),
        Some("this.search = 'zoom';"),
    )
    .unwrap();
    assert!(tree.contains("Settings › Search"), "{tree}");
}

/// The rarely used rows sit under Advanced: shown only when the fold is opened.
#[gpui::test]
fn rarely_used_settings_fold_under_advanced(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut p = props();
    p["section"] = json!("appearance");
    let tree = describe(
        cx,
        &staged,
        "frame/settings",
        true,
        p.clone(),
        updates("current", "current"),
        None,
    )
    .unwrap();
    assert!(tree.contains("st-adv-appearance") && tree.contains("Advanced"), "{tree}");
    assert!(!tree.contains("Interface font"), "{tree}");
    assert!(!tree.contains("st-layout-reset"), "{tree}");
    let tree = describe(
        cx,
        &staged,
        "frame/settings",
        true,
        p,
        updates("current", "current"),
        Some("this.advancedOpen = { appearance: true };"),
    )
    .unwrap();
    assert!(tree.contains("Interface font"), "{tree}");
    assert!(tree.contains("st-layout-reset"), "{tree}");
}

/// Appearance offers Reduce motion: the system's setting until the person
/// picks On or Off.
#[gpui::test]
fn appearance_offers_reduce_motion(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut p = props();
    p["section"] = json!("appearance");
    let mut picked = |setup: Option<&str>| {
        let tree = describe(
            cx,
            &staged,
            "frame/settings",
            true,
            p.clone(),
            updates("current", "current"),
            setup,
        )
        .unwrap();
        assert!(tree.contains("Reduce motion"), "{tree}");
        let at = tree.find("st-motion").unwrap_or_else(|| panic!("{tree}"));
        tree[at..].lines().take(8).collect::<Vec<_>>().join("
")
    };
    let system = picked(None);
    assert!(system.contains("System"), "{system}");
    let on = picked(Some("this.person = { reduce_motion: true };"));
    assert!(on.contains("\"On\""), "{on}");
}

fn appearance_values(person: Value) -> BTreeMap<String, Value> {
    let mut p = props();
    p["section"] = json!("appearance");
    let mut values = fixture(true, p, updates("current", "current"));
    values.insert("person".into(), person);
    values.insert(
        "themes".into(),
        json!([
            {"name": "graphite", "light": false, "colors": {}},
            {"name": "paper", "light": true, "colors": {}},
            {"name": "moss", "light": false, "colors": {}}
        ]),
    );
    values
}

/// The appearance tiles are titled "Light or dark", and a Theme row after them lists the themes.
#[gpui::test]
fn appearance_has_a_light_or_dark_row_and_a_theme_menu(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe_settled(
        cx,
        &staged,
        "frame/settings",
        appearance_values(json!({})),
        Some("this.themeMenu = { open: true };"),
    )
    .unwrap();
    assert!(tree.contains("Light or dark"), "{tree}");
    assert!(tree.contains("st-ap-System"), "{tree}");
    assert!(tree.contains("st-theme"), "{tree}");
    assert!(tree.contains("text \"Theme\""), "{tree}");
    for name in ["graphite", "paper", "moss"] {
        assert!(
            tree.contains(&format!("st-theme-{name}")),
            "missing {name}: {tree}"
        );
    }
}

/// The fonts are read once when the page opens: both lists keep the first answer through every render.
#[gpui::test]
fn fonts_are_read_once_per_page_opening(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut values = appearance_values(json!({}));
    values.insert("fonts".into(), json!(["Alpha Sans"]));
    values.insert("fontsAfter".into(), json!(["Beta Sans"]));
    let tree = describe_settled(
        cx,
        &staged,
        "frame/settings",
        values,
        Some("this.advancedOpen = { appearance: true }; this.fontMenus = { font: { open: true }, ui_font: { open: true } };"),
    )
    .unwrap();
    assert!(tree.contains("Alpha Sans"), "{tree}");
    assert!(!tree.contains("Beta Sans"), "fonts() was read again: {tree}");
    assert!(
        tree.matches("Alpha Sans").count() >= 2,
        "both font lists reuse the one reading: {tree}"
    );
}

/// Picking a theme runs app.theme with its name.
#[gpui::test]
fn picking_a_theme_runs_app_theme(cx: &mut TestAppContext) {
    let (effects, _) = settings_effects_after_clicks(
        cx,
        appearance_values(json!({})),
        "",
        &["st-theme", "st-theme-paper"],
        &[],
    );
    assert!(
        effects.iter().any(|(name, args)| {
            name == "run"
                && args.first() == Some(&json!("app.theme"))
                && args.get(1) == Some(&json!({"name": "paper"}))
        }),
        "{effects:?}"
    );
}

/// The menu marks the theme the current light or dark half uses.
#[gpui::test]
fn the_theme_menu_marks_the_theme_in_use(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut values = appearance_values(json!({}));
    values.get_mut("settings").unwrap()["theme"] = json!("paper");
    values.get_mut("settings").unwrap()["theme_light"] = json!("paper");
    values.get_mut("settings").unwrap()["dark"] = json!(false);
    let tree = describe_settled(
        cx,
        &staged,
        "frame/settings",
        values,
        Some("this.themeMenu = { open: true };"),
    )
    .unwrap();
    let lines: Vec<&str> = tree.lines().collect();
    let at = lines
        .iter()
        .position(|line| line.contains("st-theme-paper"))
        .unwrap_or_else(|| panic!("paper row missing: {tree}"));
    let row = lines[at..(at + 8).min(lines.len())].join("\n");
    assert!(row.contains("check.svg"), "the picked theme is marked: {row}");
    let at = lines
        .iter()
        .position(|line| line.contains("st-theme-moss"))
        .unwrap_or_else(|| panic!("moss row missing: {tree}"));
    let row = lines[at..(at + 8).min(lines.len())].join("\n");
    assert!(!row.contains("check.svg"), "another theme is not marked: {row}");
}

/// Search finds "Theme" at its new row, and "light or dark" through the light-or-dark row's words.
#[gpui::test]
fn theme_search_points_at_the_theme_row(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe(
        cx,
        &staged,
        "frame/settings",
        true,
        props(),
        updates("current", "current"),
        Some("this.search = 'theme';"),
    )
    .unwrap();
    assert!(tree.contains("Appearance › Theme"), "{tree}");
    let tree = describe(
        cx,
        &staged,
        "frame/settings",
        true,
        props(),
        updates("current", "current"),
        Some("this.search = 'appearance';"),
    )
    .unwrap();
    assert!(tree.contains("Appearance › Light or dark"), "{tree}");
}

/// Appearance draws the three theme tiles and a sample in the chosen fonts.
#[gpui::test]
fn appearance_offers_three_tiles_and_a_sample(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut p = props();
    p["section"] = json!("appearance");
    let tree = describe(
        cx,
        &staged,
        "frame/settings",
        true,
        p,
        updates("current", "current"),
        None,
    )
    .unwrap();
    for id in ["st-ap-System", "st-ap-Light", "st-ap-Dark", "st-sample"] {
        assert!(tree.contains(id), "{id}: {tree}");
    }
    assert!(tree.contains("Fix the login bug, please"), "{tree}");
    assert!(tree.contains("cargo test login"), "{tree}");
}

fn glass_values(platform: &str, live: bool) -> BTreeMap<String, Value> {
    let mut p = props();
    p["section"] = json!("appearance");
    let mut values = fixture(true, p, updates("current", "current"));
    values.insert(
        "window".into(),
        json!({"platform": platform, "glass_live": live, "titlebar": "none", "buttons": true}),
    );
    values.insert("person".into(), json!({}));
    values
}

/// Sidebar glass is a Windows row, between the theme tiles and Reduce motion.
#[gpui::test]
fn appearance_offers_sidebar_glass_on_windows_only(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    for (platform, shown) in [("windows", true), ("linux", false), ("macos", false)] {
        let tree = describe_settled(
            cx,
            &staged,
            "frame/settings",
            glass_values(platform, true),
            None,
        )
        .unwrap();
        assert_eq!(tree.contains("Sidebar glass"), shown, "{platform}: {tree}");
        assert_eq!(tree.contains("st-glass"), shown, "{platform}: {tree}");
        assert_eq!(
            tree.contains("Lets the desktop show through the sidebar."),
            shown,
            "{platform}: {tree}"
        );
        if shown {
            let theme = tree.find("st-ap-System").expect("theme tiles");
            let glass = tree.find("st-glass").expect("glass row");
            let motion = tree.find("st-motion").expect("motion row");
            assert!(theme < glass && glass < motion, "the row's place: {tree}");
        }
    }
}

/// The row's list is Off, Blur, Mica and Mica Alt; picking one writes settings.json.
#[gpui::test]
fn sidebar_glass_picks_a_material_and_writes_it(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe_settled(
        cx,
        &staged,
        "frame/settings",
        glass_values("windows", true),
        Some("this.glassMenu = { open: true };"),
    )
    .unwrap();
    for (id, label) in [
        ("st-glass-off", "Off"),
        ("st-glass-blur", "Blur"),
        ("st-glass-mica", "Mica"),
        ("st-glass-mica-alt", "Mica Alt"),
    ] {
        assert!(tree.contains(id), "missing {id}: {tree}");
        assert!(tree.contains(label), "missing {label}: {tree}");
    }
    let (writes, _) = settings_after_keys(
        cx,
        glass_values("windows", true),
        "",
        &["st-glass", "st-glass-blur"],
        None,
        "",
        &[],
    );
    assert_eq!(written_settings(&writes)["glass"], json!("blur"));
}

/// A window that can't switch its material says it applies at the next start.
#[gpui::test]
fn sidebar_glass_says_when_it_starts_again(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let live = describe_settled(
        cx,
        &staged,
        "frame/settings",
        glass_values("windows", true),
        None,
    )
    .unwrap();
    assert!(
        !live.contains("Applies when Kompitum starts again."),
        "{live}"
    );
    let next_start = describe_settled(
        cx,
        &staged,
        "frame/settings",
        glass_values("windows", false),
        None,
    )
    .unwrap();
    assert!(
        next_start.contains("Applies when Kompitum starts again."),
        "{next_start}"
    );
}

fn notification_values(platform: &str, taken: bool, person: Value) -> BTreeMap<String, Value> {
    let mut p = props();
    p["section"] = json!("notifications");
    let mut values = fixture(true, p, updates("current", "current"));
    values.insert(
        "window".into(),
        json!({"platform": platform, "summon_taken": taken}),
    );
    values.insert("person".into(), person);
    values
}

/// The summon key and Windows notifications are Windows rows, with their words.
#[gpui::test]
fn windows_shows_the_summon_key_and_notification_rows(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    for (platform, shown) in [("windows", true), ("linux", false), ("macos", false)] {
        let tree = describe_settled(
            cx,
            &staged,
            "frame/settings",
            notification_values(platform, false, json!({})),
            None,
        )
        .unwrap();
        assert_eq!(tree.contains("Summon Kompitum"), shown, "{platform}: {tree}");
        assert_eq!(tree.contains("Windows notifications"), shown, "{platform}: {tree}");
        if shown {
            for words in [
                "Brings Kompitum forward with the keyboard in the composer, from any app.",
                "When something needs you and Kompitum isn't in front.",
                "Ctrl+Alt+K",
            ] {
                assert!(tree.contains(words), "missing {words}: {tree}");
            }
        }
    }
}

/// The summon dropdown lists all three choices.
#[gpui::test]
fn the_summon_key_dropdown_offers_every_choice(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe_settled(
        cx,
        &staged,
        "frame/settings",
        notification_values("windows", false, json!({})),
        Some("this.summonKeyMenu.open = true;"),
    )
    .unwrap();
    for choice in ["Ctrl+Alt+K", "Ctrl+Alt+Space", "Off"] {
        assert!(tree.contains(choice), "missing {choice}: {tree}");
    }
}

/// Picking a choice writes settings.json's summon_key.
#[gpui::test]
fn picking_a_summon_key_writes_it(cx: &mut TestAppContext) {
    let (writes, _) = settings_after_keys(
        cx,
        notification_values("windows", false, json!({})),
        "this.summonKeyPick('off', cx);",
        &[],
        None,
        "",
        &[],
    );
    assert_eq!(written_settings(&writes)["summon_key"], json!("off"));
}

/// A key another app holds says so, in the blocked colour.
#[gpui::test]
fn a_taken_summon_key_says_another_app_has_it(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let values = notification_values("windows", true, json!({}));
    let blocked = values["theme"]["blocked"].as_str().unwrap().to_string();
    let tree = describe_settled(cx, &staged, "frame/settings", values, None).unwrap();
    let line = tree
        .lines()
        .position(|line| line.contains("Another app is using this key. Pick another."))
        .unwrap_or_else(|| panic!("the taken-key line is drawn: {tree}"));
    let lines: Vec<&str> = tree.lines().collect();
    let near = lines[line.saturating_sub(4)..=line].join("\n");
    assert!(near.contains(&blocked), "it is written in the blocked colour: {near}");
    assert!(
        !tree.contains("Brings Kompitum forward with the keyboard in the composer, from any app."),
        "the taken line replaces the usual one: {tree}"
    );
}


fn about_values(person: Value, workers: Value) -> BTreeMap<String, Value> {
    let mut p = props();
    p["section"] = json!("about");
    let mut values = fixture(true, p, updates("current", "current"));
    values.insert("person".into(), person);
    values.insert("workersJson".into(), workers);
    values
}

fn export_person() -> Value {
    json!({
        "zoom": 1.5,
        "summon_key": "ctrl+alt+space",
        "theme": "moss",
        "lead": {"model": "opus", "api_key": "keep-me"},
        "connect": {"grokbot": {"name": "Grok", "token": "secret-token", "nested": {"secret": "x"}}},
        "nested": {"keep": true, "password": "hidden"},
        "colors": {"accent": "#ff8800"}
    })
}

fn export_workers() -> Value {
    agent_workers(
        json!(["cc", "go", "mine"]),
        json!([
            agent_worker("cc", "Command Code", "command_code", "cc-route", "ready", "", 0),
            agent_worker("go", "opencode Go", "opencode", "go-route", "ready", "", 0),
            json!({
                "key": "mine", "label": "My Worker", "cli": "opencode", "route": "mine-route",
                "state": "ready", "until_ms": 0, "source": null, "window": "", "message": "",
                "sentence": "", "refusals": 0,
                "spend": {"five_hours": {"runs": 3, "cost_usd": 0.5}, "week": {"runs": 3, "cost_usd": 0.5}, "month": {"runs": 3, "cost_usd": 0.5}}
            })
        ]),
    )
}

/// Export writes one JSON file: the settings without their secrets, and only the workers someone
/// added, with their order and the mode.
#[gpui::test]
fn export_writes_the_person_without_secrets_and_the_added_workers(cx: &mut TestAppContext) {
    let mut values = about_values(export_person(), export_workers());
    values.insert("pickSavePath".into(), json!("/fixture/kompitum-settings.json"));
    let (writes, _) = settings_after_keys(cx, values, "", &["st-export"], None, "", &[]);
    let text = writes.last().and_then(Value::as_str).unwrap_or_else(|| panic!("nothing was written: {writes:?}"));
    let doc: Value = serde_json::from_str(text).unwrap();
    assert_eq!(doc["kompitum_settings"], json!(1));
    assert!(doc["exported"].as_str().is_some_and(|time| time.contains('T')), "{doc}");
    assert_eq!(doc["person"]["zoom"], json!(1.5));
    assert_eq!(doc["person"]["summon_key"], json!("ctrl+alt+space"), "a setting, not a secret");
    assert_eq!(doc["person"]["nested"]["keep"], json!(true));
    assert_eq!(doc["person"]["colors"]["accent"], json!("#ff8800"));
    let exported = text.to_lowercase();
    for secret in ["api_key", "keep-me", "secret-token", "password", "\"secret\"", "hidden"] {
        assert!(!exported.contains(secret), "{secret:?} stayed in: {text}");
    }
    assert_eq!(doc["workers"]["order"], json!(["cc", "go", "mine"]));
    assert_eq!(doc["workers"]["mode"], json!("auto"));
    assert_eq!(
        doc["workers"]["added"],
        json!([{"key": "mine", "cli": "opencode", "label": "My Worker", "route": "mine-route"}])
    );
}

fn import_document() -> Value {
    json!({
        "kompitum_settings": 1,
        "exported": "2026-11-15T00:00:00Z",
        "person": {
            "zoom": 2,
            "theme": "paper",
            "lead": {"model": "opus"},
            "connect": {"grokbot": {"name": "New name", "token": "from-file"}}
        },
        "workers": {
            "order": ["go", "mine", "cc"],
            "mode": "auto",
            "added": [{"key": "mine", "cli": "opencode", "label": "Mine", "route": "mine-route"}]
        }
    })
}

fn import_values(document: Value) -> BTreeMap<String, Value> {
    let person = json!({
        "zoom": 1,
        "theme": "ink",
        "lead": {"model": "sonnet", "api_key": "keep-me"},
        "connect": {"grokbot": {"name": "Old name", "token": "keep-token"}}
    });
    let workers = agent_workers(
        json!(["cc", "go"]),
        json!([
            agent_worker("cc", "Command Code", "command_code", "cc-route", "ready", "", 0),
            agent_worker("go", "opencode Go", "opencode", "go-route", "ready", "", 0)
        ]),
    );
    let mut values = about_values(person, workers);
    values.insert("importFile".into(), json!(document.to_string()));
    values.insert("pickedFiles".into(), json!(["/fixture/import.json"]));
    values
}

/// The import sheet lists the person settings that differ and the workers to add or move.
#[gpui::test]
fn the_import_sheet_lists_what_will_change(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe_settled(
        cx,
        &staged,
        "frame/settings",
        import_values(import_document()),
        Some("this.openImport(cx);"),
    )
    .unwrap();
    for line in [
        "zoom: 1 → 2",
        "Add worker Mine",
        "Move worker opencode Go: 2 → 1",
    ] {
        assert!(tree.contains(line), "missing {line:?}: {tree}");
    }
    assert!(tree.contains("Import"), "{tree}");
    assert!(tree.contains("Cancel"), "{tree}");
    assert!(!tree.contains("keep-token"), "a secret is not in the sheet: {tree}");
    assert!(!tree.contains("from-file"), "an exported secret is ignored: {tree}");
}

/// A file that isn't a Kompitum settings file says so, and shows no sheet.
#[gpui::test]
fn a_file_that_is_not_settings_says_so(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut values = import_values(json!({"kompitum_settings": 2}));
    values.insert("importFile".into(), json!("not json at all"));
    let tree = describe_settled(
        cx,
        &staged,
        "frame/settings",
        values,
        Some("this.openImport(cx);"),
    )
    .unwrap();
    assert!(tree.contains("This isn't a Kompitum settings file."), "{tree}");
    assert!(!tree.contains("st-import-sheet"), "{tree}");
}

/// Import writes the merged person settings through the page's change, keeping the current secret
/// fields, then adds the new worker, the order and the mode.
#[gpui::test]
fn import_writes_the_person_then_the_worker_commands(cx: &mut TestAppContext) {
    let (effects, found) = settings_effects_after_clicks(
        cx,
        import_values(import_document()),
        "",
        &["st-import", "st-import-confirm"],
        &["st-import-sheet"],
    );
    assert_eq!(found, vec![("st-import-sheet".into(), false)]);
    let writes: Vec<Value> = effects
        .iter()
        .filter(|(name, _)| name == "writeTextFile")
        .filter_map(|(_, args)| args.get(1).cloned())
        .collect();
    let person = written_settings(&writes);
    assert_eq!(person["zoom"], json!(2));
    assert_eq!(person["theme"], json!("paper"));
    assert_eq!(person["lead"]["model"], json!("opus"));
    assert_eq!(person["lead"]["api_key"], json!("keep-me"), "the current secret stays");
    assert_eq!(person["connect"]["grokbot"]["name"], json!("New name"));
    assert_eq!(person["connect"]["grokbot"]["token"], json!("keep-token"));
    let commands: Vec<Vec<Value>> = effects
        .iter()
        .filter(|(name, _)| name == "kom")
        .filter_map(|(_, args)| args.first()?.as_array().cloned())
        .collect();
    for want in [
        json!(["workers", "--add", "mine", "--cli", "opencode", "--label", "Mine", "--route", "mine-route"]),
        json!(["workers", "--order", "go,mine,cc"]),
        json!(["workers", "--mode", "auto"]),
    ] {
        assert!(commands.contains(&want.as_array().cloned().unwrap_or_default()), "missing {want}: {commands:?}");
    }
}

fn failed_save_values() -> BTreeMap<String, Value> {
    let mut p = props();
    p["section"] = json!("notifications");
    let mut values = fixture(true, p, updates("current", "current"));
    values.insert("person".into(), json!({}));
    values.insert("settingsWriteFail".into(), json!("disk full"));
    values
}

/// A setting whose write fails leaves the switch as it was, says why at the top of the page, and
/// posts the same words as an error toast.
#[gpui::test]
fn a_setting_that_cannot_be_saved_says_so_and_toasts(cx: &mut TestAppContext) {
    let (effects, found) = settings_effects_after_clicks(
        cx,
        failed_save_values(),
        "",
        &["st-sounds"],
        &["st-save-error"],
    );
    assert_eq!(found, vec![("st-save-error".into(), true)]);
    assert!(
        effects.iter().any(|(name, args)| {
            name == "notify"
                && args.first().is_some_and(|n| {
                    n["level"] == json!("error")
                        && n["title"]
                            .as_str()
                            .is_some_and(|title| title.starts_with("Couldn't save the setting: ") && title.contains("disk full"))
                })
        }),
        "the failure is a toast too: {effects:?}"
    );
    let tree = describe_settled(
        cx,
        &Staged::new(),
        "frame/settings",
        failed_save_values(),
        Some("this.saveError = 'disk full';"),
    )
    .unwrap();
    assert!(tree.contains("Couldn't save the setting: disk full"), "{tree}");
}

/// "Copied." clears two seconds after it was set, whether or not an update check is running.
#[gpui::test]
fn copied_clears_two_seconds_after_it_was_set(cx: &mut TestAppContext) {
    use gpui::{IntoElement as _, Render};
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut p = props();
    p["section"] = json!("about");
    let mut values = fixture(true, p, updates("current", "current"));
    values.insert("person".into(), json!({}));
    let calls = Rc::new(RefCell::new(Calls::default()));
    let policy = Policy::new()
        .with_storage_path(staged.0.join("store-copied.json"))
        .with_capabilities(Capabilities::new().storage(true).read_roots([staged.0.clone()]))
        .with_host_module(module(values, calls.clone()))
        .unwrap();
    gpui_shell::policy::set_default(policy);
    let runtime = ShellRuntime::new_isolated().unwrap();
    let root = staged.wrapper("frame/settings", "this.copiedAt = Date.now() - 2100;");
    let window = cx.add_window(|window, cx| Mounted(runtime.try_load(&root, window, cx).unwrap()));
    let mut visual = VisualTestContext::from_window(*window.deref(), cx);
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear(cx));
    visual.run_until_parked();
    assert!(
        visual.debug_bounds("st-copied").is_some(),
        "Copied is drawn before its timer runs"
    );
    visual.executor().advance_clock(std::time::Duration::from_millis(350));
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear(cx));
    visual.run_until_parked();
    assert!(
        visual.debug_bounds("st-copied").is_none(),
        "Copied clears two seconds after it was set"
    );
    assert!(calls.borrow().unexpected.is_empty(), "{:?}", calls.borrow().unexpected);
    drop(visual);
    cx.run_until_parked();
    window.update(cx, |_, window, _| window.remove_window()).ok();
    cx.run_until_parked();
}

/// A folder picker that cannot open says why under the lead's folder row.
#[gpui::test]
fn a_folder_picker_that_cannot_open_says_so(cx: &mut TestAppContext) {
    let mut values = lead_values("claude");
    values.insert("pickFolderFail".into(), json!("no picker here"));
    let (effects, found) = settings_effects_after_clicks(
        cx,
        values,
        "",
        &["st-lead-folder"],
        &["st-folder-error"],
    );
    assert_eq!(found, vec![("st-folder-error".into(), true)]);
    assert!(
        effects.iter().any(|(name, _)| name == "pickFolder"),
        "the picker was tried: {effects:?}"
    );
    let tree = describe_settled(
        cx,
        &Staged::new(),
        "frame/settings",
        lead_values("claude"),
        Some("this.folderError = 'no picker here';"),
    )
    .unwrap();
    assert!(
        tree.contains("Couldn't open the folder picker: no picker here"),
        "{tree}"
    );
}

/// Copy diagnostics puts the lines on the clipboard, with the home folder and tokens taken out, and
/// says "Copied." beside the button.
#[gpui::test]
fn copy_diagnostics_puts_the_block_on_the_clipboard(cx: &mut TestAppContext) {
    let token = "abcdefghijklmnopqrstuvwxyz012345";
    let workers = agent_workers(
        json!(["cc", "go"]),
        json!([
            agent_worker("/fixture/repo", "/fixture/repo", "command_code", "cc-route", "ready", "", 0),
            agent_worker(token, token, "opencode", "go-route", "out", "5-hour", 4102444800000),
            agent_worker("notes", "\\Fixture\\notes", "command_code", "notes-route", "ready", "", 0),
            agent_worker("near", "/fixtures/near", "command_code", "near-route", "ready", "", 0)
        ]),
    );
    let mut values = about_values(json!({"lead": {"model": "opus"}}), workers);
    values.insert("sharedValues".into(), json!({"claude.version": "2.0.14"}));
    values.insert("system".into(), json!({"name": "Windows", "version": "10.0.22631"}));
    values.insert(
        "lead".into(),
        json!({"state": "waiting", "permission_mode": "default", "started": true, "terminal": "hidden"}),
    );
    let (effects, found) = settings_effects_after_clicks(
        cx,
        values,
        "",
        &["st-copy-diagnostics"],
        &["st-copied"],
    );
    assert_eq!(found, vec![("st-copied".into(), true)]);
    let text = effects
        .iter()
        .find(|(name, _)| name == "copyText")
        .and_then(|(_, args)| args.first())
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("nothing was copied: {effects:?}"));
    for line in [
        "kompitum-app: 0.1.0",
        "kompitum-core: 0.1.5",
        "os: Windows 10.0.22631",
        "claude: 2.0.14",
        "lead: waiting · default",
        "workers:",
        "~/repo: ready",
        "~\\notes: ready",
        "/fixtures/near: ready",
        "<hidden>: out until 2100-01-01",
        "theme: ",
        "reduce_motion: false",
        "quiet_while_fullscreen: true",
    ] {
        assert!(text.contains(line), "missing {line:?}: {text}");
    }
    assert!(!text.contains("/fixture/"), "the home path is ~: {text}");
    assert!(!text.contains("\\Fixture"), "the home path is ~ with either slash and any case: {text}");
    assert!(!text.contains(token), "the token is hidden: {text}");
}
