//! Render tests for the Limits sheet: the context and Compact now, Claude's two windows, and the workers.
#![allow(unused_imports)]

use super::*;

fn now_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

/// The lead's usage: the context against the compact point of 444000, and Claude's two windows as given.
fn lead_usage(context_tokens: u64, five_hour: Value, seven_day: Value) -> Value {
    json!({
        "context_tokens": context_tokens,
        "context_window": 1000000,
        "compact_at": 444000,
        "cache_ttl": null,
        "cache_expires_at": null,
        "five_hour": five_hour,
        "seven_day": seven_day,
        "session_id": null,
        "seen_ms": 0
    })
}

/// A status with no workers: the sheet's worker section is empty but still read.
fn no_workers() -> Value {
    json!({"workers": {"chain": [], "next": null, "workers": []}})
}

/// The values the sheet reads: the lead waiting with `usage`, the status `kom status` gives, and the popup's props.
fn sheet_values(usage: Value, status: Value, props: Value) -> BTreeMap<String, Value> {
    let mut values = fixture(false, props, updates("current", "current"));
    values.insert("statusJson".into(), status);
    values.insert(
        "lead".into(),
        json!({
            "state": "waiting",
            "ask": null,
            "started": true,
            "terminal": "hidden",
            "reveal": null,
            "reveal_why": null,
            "conversation": { "version": 1, "length": 0 },
            "model": null,
            "usage": usage,
            "compacting": false
        }),
    );
    values
}

fn sheet_draws(cx: &mut TestAppContext, values: BTreeMap<String, Value>) -> String {
    let staged = Staged::new();
    describe_settled(cx, &staged, "mods/limits", values, None).unwrap()
}

/// The context bar, and Compact now only when the popup has a terminal to run it in.
#[gpui::test]
fn the_sheet_shows_the_context_and_compact_now(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let usage = lead_usage(271000, Value::Null, Value::Null);
    let tree = sheet_draws(
        cx,
        sheet_values(usage.clone(), no_workers(), json!({"overlay": "o1", "terminal": "t1"})),
    );
    for shown in [
        "limits-context",
        "271K of 444K",
        "limits-compact",
        "Compact now",
        "Kompitum tidies up by itself while you're away.",
    ] {
        assert!(tree.contains(shown), "missing {shown}: {tree}");
    }
    let bare = sheet_draws(cx, sheet_values(usage, no_workers(), json!({"overlay": "o1"})));
    assert!(bare.contains("limits-context"), "{bare}");
    assert!(!bare.contains("limits-compact"), "{bare}");
}

/// Claude's two windows are bars, each with a pace mark from its reset; with no usage the sheet says when they show.
#[gpui::test]
fn claudes_windows_have_bars_and_a_pace_mark(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let now = now_seconds();
    let usage = lead_usage(
        271000,
        json!({"used_percentage": 41, "resets_at": now + 2 * 3600}),
        json!({"used_percentage": 73, "resets_at": now + 33 * 3600}),
    );
    let tree = sheet_draws(cx, sheet_values(usage, no_workers(), json!({"overlay": "o1"})));
    for shown in [
        "limits-claude-5h",
        "limits-claude-week",
        "5-hour limit",
        "Week",
        "41%",
        "limits-claude-5h-pace",
        "limits-claude-week-pace",
    ] {
        assert!(tree.contains(shown), "missing {shown}: {tree}");
    }
    let none = sheet_draws(cx, sheet_values(Value::Null, no_workers(), json!({"overlay": "o1"})));
    assert!(none.contains("limits-claude-none"), "{none}");
    assert!(!none.contains("limits-claude-5h"), "{none}");
}

/// Each worker shows its state and its week's cost, in the chain's order; the header sums the week.
#[gpui::test]
fn the_workers_show_state_and_cost_in_chain_order(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let status = json!({"workers": {"chain": ["cc", "go", "haiku"], "next": "go", "workers": [
        {"key": "cc", "label": "Command Code", "cli": "command_code", "state": "out", "until_ms": 4102444800000i64,
         "spend": {"week": {"cost_usd": 0.0, "runs": 0}}},
        {"key": "go", "label": "opencode Go", "cli": "opencode", "state": "ready", "until_ms": 0,
         "spend": {"week": {"cost_usd": 0.0, "runs": 0}}},
        {"key": "haiku", "label": "Haiku", "cli": "haiku", "state": "ready", "until_ms": 0,
         "spend": {"week": {"cost_usd": 8.878966, "runs": 22}}}
    ]}});
    let tree = sheet_draws(cx, sheet_values(Value::Null, status, json!({"overlay": "o1"})));
    for shown in [
        "limits-worker-cc",
        "Out until",
        "limits-worker-go",
        "Next up",
        "limits-worker-haiku",
        "$8.88 this week · 22 runs",
        "Nothing this week",
        "limits-spend-total",
        "$8.88 this week",
    ] {
        assert!(tree.contains(shown), "missing {shown}: {tree}");
    }
    let cc = tree.find("limits-worker-cc").unwrap();
    let go = tree.find("limits-worker-go").unwrap();
    assert!(cc < go, "cc's row comes before go's: {tree}");
}

/// A worker whose CLI reports meters draws a bar for each present meter. The fake exec answers `opencode`
/// with its nested report: rolling 0%, weekly 19%, monthly 96%.
#[gpui::test]
fn a_worker_cli_report_draws_its_meters(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let status = json!({"workers": {"chain": ["go"], "next": "go", "workers": [
        {"key": "go", "label": "opencode Go", "cli": "opencode", "state": "ready", "until_ms": 0,
         "spend": {"week": {"cost_usd": 1.5, "runs": 3}}}
    ]}});
    // OpenCode's entry names the report it is asked for; the fake answers any `opencode` call.
    let mut catalog = agent_catalog();
    catalog[2]["usage"] = json!(["stats", "--format", "json"]);
    let mut values = sheet_values(Value::Null, status, json!({"overlay": "o1"}));
    values.insert("knownAgents".into(), catalog);
    let tree = sheet_draws(cx, values);
    for shown in ["limits-worker-go-5h", "limits-worker-go-week", "limits-worker-go-month", "19%", "96%"] {
        assert!(tree.contains(shown), "missing {shown}: {tree}");
    }
}
