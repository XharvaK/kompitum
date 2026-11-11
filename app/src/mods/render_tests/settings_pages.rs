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
    for meter in ["5H 100%", "Week 19%", "Month 96%"] {
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
        .position(|line| line.contains("text \"Month 96%\""))
        .unwrap_or_else(|| panic!("no month meter: {tree}"));
    assert!(
        lines[at - 1].contains(working),
        "96% is working: {}",
        lines[at - 1]
    );
    let at = lines
        .iter()
        .position(|line| line.contains("text \"5H 100%\""))
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
        let base = row.replace(&format!(":hover(.bg[Str(\"{hover}\")])"), "");
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
    let tree = describe_settled(cx, &staged, "frame/settings", values, None).unwrap();
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

