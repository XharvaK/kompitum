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
