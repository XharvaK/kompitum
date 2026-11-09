//! Render tests for the composer's box (audit/REWORK-2026-11-09.md, C1 to C6): see the parent module for the
//! helpers.
#![allow(unused_imports)]

use super::*;

/// One box: no placeholder, "+" to attach, the lead's model, round Send;
/// no keyboard button, no plan ring and no mail button.
#[gpui::test]
fn the_composer_is_one_box_with_attach_model_and_send(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut values = fixture(true, props(), updates("current", "current"));
    values.insert("knownAgents".into(), agent_catalog());
    let tree = describe_settled(cx, &staged, "mods/composer", values, None).unwrap();
    for id in ["composer-box", "composer-attach", "composer-model", "composer-send"] {
        assert!(tree.contains(id), "{id}: {tree}");
    }
    for gone in ["composer-type", "keyboard.svg", "Message the lead", "composer-plan", "composer-mail"] {
        assert!(!tree.contains(gone), "{gone}: {tree}");
    }
}

fn composer_person(model: Option<&str>, effort: Option<&str>) -> Value {
    let mut lead = serde_json::Map::new();
    lead.insert("program".into(), json!("claude"));
    if let Some(model) = model {
        lead.insert("model".into(), json!(model));
    }
    if let Some(effort) = effort {
        lead.insert("effort".into(), json!(effort));
    }
    json!({ "lead": lead })
}

fn composer_tree(cx: &mut TestAppContext, person: Value, setup: Option<&str>) -> String {
    let staged = Staged::new();
    let mut values = fixture(true, props(), updates("current", "current"));
    values.insert("knownAgents".into(), agent_catalog());
    values.insert("person".into(), person);
    describe_settled(cx, &staged, "mods/composer", values, setup).unwrap()
}

/// The trigger shows the saved model and the reasoning's word (Low, not "low effort").
#[gpui::test]
fn the_model_trigger_shows_the_saved_model_and_level(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let tree = composer_tree(cx, composer_person(Some("opus"), Some("high")), None);
    assert!(tree.contains("Opus"), "{tree}");
    assert!(tree.contains("High"), "{tree}");
    assert!(!tree.contains("high effort"), "{tree}");
    assert!(
        tree.contains("The lead's model and reasoning, for its next start"),
        "{tree}"
    );
}

#[gpui::test]
fn the_model_trigger_asks_to_choose_when_none_is_saved(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let tree = composer_tree(cx, composer_person(None, None), None);
    assert!(tree.contains("Choose a model"), "{tree}");
}

/// Opening the card lists exactly Claude's four models and starts on that list.
#[gpui::test]
fn the_models_view_lists_fable_opus_sonnet_haiku(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let tree = composer_tree(
        cx,
        composer_person(Some("sonnet"), Some("low")),
        Some("this.modelOpen = true;"),
    );
    for id in ["fable", "opus", "sonnet", "haiku"] {
        assert!(
            tree.contains(&format!("composer-model-{id}")),
            "missing {id}: {tree}"
        );
    }
    for gone in ["composer-model-default", "composer-model-best", "composer-effort"] {
        assert!(!tree.contains(gone), "{gone}: {tree}");
    }
}

/// Picking a model (a click, not setting pickerView) switches to the effort card.
#[gpui::test]
fn picking_a_model_opens_the_effort_view(cx: &mut TestAppContext) {
    let found = composer_laid_out_after_clicks(
        cx,
        composer_person(Some("sonnet"), None),
        "this.modelOpen = true;",
        &["composer-model-opus"],
        &["composer-effort", "composer-effort-reset"],
    );
    assert!(found[0].1, "the effort view should show after picking a model: {found:?}");
    assert!(!found[1].1, "reset stays hidden when no level is saved: {found:?}");
}

#[gpui::test]
fn the_effort_reset_shows_only_when_a_level_is_saved(cx: &mut TestAppContext) {
    let found = composer_laid_out_after_clicks(
        cx,
        composer_person(Some("sonnet"), Some("high")),
        "this.modelOpen = true;",
        &["composer-model-sonnet"],
        &["composer-effort", "composer-effort-reset"],
    );
    assert!(found[0].1, "the effort view should show: {found:?}");
    assert!(found[1].1, "reset should show when a level is saved: {found:?}");
}

#[gpui::test]
fn the_first_pick_card_shows_when_no_model_is_saved(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut values = fixture(false, props(), updates("current", "current"));
    values.insert("knownAgents".into(), agent_catalog());
    values.insert("person".into(), composer_person(None, None));
    let tree = describe_settled(cx, &staged, "mods/composer", values, None).unwrap();
    assert!(tree.contains("composer-first"), "{tree}");
    assert!(tree.contains("Which model should the lead use?"), "{tree}");
    assert!(
        tree.contains("You can change it any time from the model menu below."),
        "{tree}"
    );
    for id in ["fable", "opus", "sonnet", "haiku"] {
        assert!(
            tree.contains(&format!("composer-first-{id}")),
            "missing {id}: {tree}"
        );
    }
    assert!(tree.contains("composer-first-start"), "{tree}");
    assert!(tree.contains("Start the lead"), "{tree}");
    for gone in ["composer-send", "composer-empty", "composer-open"] {
        assert!(!tree.contains(gone), "{gone}: {tree}");
    }
    assert!(
        !tree.contains(":id[Str(\"composer-model\")]"),
        "the picker stays hidden: {tree}"
    );
}

#[gpui::test]
fn start_stays_inert_until_a_model_row_is_clicked(cx: &mut TestAppContext) {
    let before = composer_run_effects_after_clicks(
        cx,
        composer_person(None, None),
        "",
        &["composer-first-start"],
    );
    assert!(
        !before.iter().any(|args| args.first() == Some(&json!("lead.open"))),
        "Start must do nothing until a model is picked: {before:?}"
    );
    let after = composer_run_effects_after_clicks(
        cx,
        composer_person(None, None),
        "",
        &["composer-first-opus", "composer-first-start"],
    );
    assert!(
        after.iter().any(|args| args.first() == Some(&json!("lead.open"))),
        "Start after picking a model should run lead.open: {after:?}"
    );
}

fn conversation_values(items: Value, terminal: &str, reveal: Value, why: Option<&str>) -> BTreeMap<String, Value> {
    let mut values = fixture(true, props(), updates("current", "current"));
    values.insert("knownAgents".into(), agent_catalog());
    let length = items.as_array().map(|a| a.len()).unwrap_or(0);
    values.insert("conversation".into(), items.clone());
    values.insert(
        "lead".into(),
        json!({
            "state": "waiting",
            "ask": null,
            "terminal": terminal,
            "reveal": reveal,
            "reveal_why": why,
            "conversation": { "version": 1, "length": length }
        }),
    );
    values
}

fn composer_live_run_after_clicks(
    cx: &mut TestAppContext,
    values: BTreeMap<String, Value>,
    setup: &str,
    clicks: &[&'static str],
) -> Vec<Vec<Value>> {
    use gpui::{Modifiers, MouseButton};
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let calls = Rc::new(RefCell::new(Calls::default()));
    let policy = Policy::new()
        .with_storage_path(staged.0.join("store-chat-view.json"))
        .with_capabilities(Capabilities::new().storage(true).read_roots([staged.0.clone()]))
        .with_host_module(module(values, calls.clone()))
        .unwrap();
    gpui_shell::policy::set_default(policy);
    let runtime = ShellRuntime::new_isolated().unwrap();
    let root = staged.wrapper("mods/composer", setup);
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
    drop(visual);
    cx.run_until_parked();
    window.update(cx, |_, window, _| window.remove_window()).ok();
    cx.run_until_parked();
    let calls = calls.borrow();
    assert!(calls.unexpected.is_empty(), "{:?}", calls.unexpected);
    calls
        .effects
        .iter()
        .filter(|(name, _)| name == "run" || name == "kom")
        .map(|(_, args)| args.clone())
        .collect()
}

/// Each conversation kind draws: the owner's words, the assistant's Markdown, a tool summary, a notice.
#[gpui::test]
fn each_conversation_item_kind_draws(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let items = json!([
        {"kind": "owner", "id": "o1", "text": "Look at lead.rs"},
        {"kind": "assistant", "id": "a1", "text": "I'll read the file.", "done": true},
        {"kind": "tool", "id": "u1", "name": "Read", "summary": "Read app/src/lead.rs", "state": "done", "result": "pub fn fold"},
        {"kind": "notice", "id": "n1", "text": "Signed in."}
    ]);
    let tree = describe_settled(
        cx,
        &staged,
        "mods/composer",
        conversation_values(items, "hidden", Value::Null, None),
        None,
    )
    .unwrap();
    for needle in [
        "composer-item-o1",
        "Look at lead.rs",
        "composer-item-a1",
        "I'll read the file.",
        "composer-item-u1",
        "Read app/src/lead.rs",
        "composer-item-n1",
        "Signed in.",
        "lead-terminal",
    ] {
        assert!(tree.contains(needle), "missing {needle}: {tree}");
    }
    assert!(!tree.contains("Ask the lead anything."), "{tree}");
}

/// Consecutive finished tools collapse to one line until it is opened.
#[gpui::test]
fn finished_tools_fold_into_one_line(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let items = json!([
        {"kind": "tool", "id": "u1", "name": "Read", "summary": "Read a.rs", "state": "done"},
        {"kind": "tool", "id": "u2", "name": "Bash", "summary": "Ran `cargo test`", "state": "done"}
    ]);
    let tree = describe_settled(
        cx,
        &staged,
        "mods/composer",
        conversation_values(items, "hidden", Value::Null, None),
        None,
    )
    .unwrap();
    assert!(tree.contains("Used 2 tools"), "{tree}");
    assert!(tree.contains("composer-tools-used"), "{tree}");
    assert!(!tree.contains("Read a.rs"), "{tree}");
    assert!(!tree.contains("Ran `cargo test`"), "{tree}");
}

/// The terminal toggle is on the panel and runs lead.terminal.
#[gpui::test]
fn the_terminal_toggle_shows_and_runs_lead_terminal(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let items = json!([]);
    let tree = describe_settled(
        cx,
        &staged,
        "mods/composer",
        conversation_values(items.clone(), "hidden", Value::Null, None),
        None,
    )
    .unwrap();
    assert!(tree.contains("lead-terminal"), "{tree}");
    assert!(tree.contains("Ask the lead anything."), "{tree}");
    let effects = composer_live_run_after_clicks(
        cx,
        conversation_values(items, "hidden", Value::Null, None),
        "",
        &["lead-terminal"],
    );
    assert!(
        effects.iter().any(|args| args.first() == Some(&json!("lead.terminal"))),
        "the toggle should run lead.terminal: {effects:?}"
    );
}

/// Sending a bare /model is still kom send; the hook's auto reveal draws the note.
#[gpui::test]
fn a_bare_model_send_reveals(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let items = json!([{"kind": "owner", "id": "o1", "text": "/model"}]);
    let tree = describe_settled(
        cx,
        &staged,
        "mods/composer",
        conversation_values(
            items,
            "shown",
            json!("auto"),
            Some("Claude is showing its /model menu."),
        ),
        None,
    )
    .unwrap();
    assert!(tree.contains("lead-terminal"), "{tree}");
    assert!(tree.contains("composer-reveal-note"), "{tree}");
    assert!(tree.contains("Claude is showing its /model menu."), "{tree}");
    assert!(
        !tree.contains("composer-item-o1"),
        "the conversation is not drawn while the terminal is shown: {tree}"
    );
    let sent = composer_sends_after_keys(
        cx,
        "this.text.set_value('/model'); this.text.set_submit_on_enter(true); this.setHistory(false, cx); this.text.focus();",
        "enter",
    );
    assert!(
        sent.iter().any(|args| args.get(2).and_then(Value::as_str).is_some_and(|text| text.trim() == "/model")),
        "a bare /model still goes through kom send: {sent:?}"
    );
}
