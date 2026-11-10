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
                tree.contains("Home folder"),
                "{platform}: keyboard opens General: {tree}"
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

