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
    conversation_values_started(items, terminal, reveal, why, true)
}

fn conversation_values_started(
    items: Value,
    terminal: &str,
    reveal: Value,
    why: Option<&str>,
    started: bool,
) -> BTreeMap<String, Value> {
    let mut values = fixture(true, props(), updates("current", "current"));
    values.insert("knownAgents".into(), agent_catalog());
    let length = items.as_array().map(|a| a.len()).unwrap_or(0);
    values.insert("conversation".into(), items.clone());
    values.insert(
        "lead".into(),
        json!({
            "state": "waiting",
            "ask": null,
            "started": started,
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
        .filter(|(name, _)| {
            name == "run"
                || name == "kom"
                || name == "pickFiles"
                || name == "pickFolder"
                || name == "writeTextFile"
        })
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
    ] {
        assert!(tree.contains(needle), "missing {needle}: {tree}");
    }
    assert!(!tree.contains("lead-terminal"), "{tree}");
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

/// No terminal toggle: the conversation has no button to show Claude's terminal (the automatic reveals stay).
#[gpui::test]
fn the_conversation_has_no_terminal_toggle(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe_settled(
        cx,
        &staged,
        "mods/composer",
        conversation_values(json!([]), "hidden", Value::Null, None),
        None,
    )
    .unwrap();
    assert!(!tree.contains("lead-terminal"), "{tree}");
    // The empty state is the catalog name alone, outside the scroll column.
    assert!(tree.contains("composer-conversation-empty"), "{tree}");
    assert!(tree.contains("Claude Code"), "{tree}");
    assert!(!tree.contains("Ask the lead anything."), "{tree}");
}

/// "+" opens the system's file picker on a click: its handler takes the click's own context.
#[gpui::test]
fn the_attach_button_opens_the_file_picker_on_a_click(cx: &mut TestAppContext) {
    let effects = composer_live_run_after_clicks(
        cx,
        conversation_values(json!([]), "hidden", Value::Null, None),
        "",
        &["composer-attach"],
    );
    assert!(
        effects.iter().any(|args| args.first() == Some(&json!("pickFiles"))),
        "+ should open the file picker: {effects:?}"
    );
}

/// Send on a click sends the typed words.
#[gpui::test]
fn send_on_a_click_sends_the_typed_words(cx: &mut TestAppContext) {
    let effects = composer_live_run_after_clicks(
        cx,
        conversation_values(json!([]), "hidden", Value::Null, None),
        "this.text.set_value('hello');",
        &["composer-send"],
    );
    assert!(
        effects.iter().any(|args| args.first() == Some(&json!(["send", "t1", "hello"]))),
        "Send should send the typed words: {effects:?}"
    );
}

/// The setup that opens the effort card as the picker does. The catalog is handed over first: the first load
/// hasn't run when a setup runs, and the slider needs the levels.
fn effort_card_setup() -> String {
    format!(
        "this.agents = {}; this.modelOpen = true; this.pickerView = 'effort'; this.ensureEffortSlider(this.leadProgram());",
        agent_catalog()
    )
}

/// The effort card explains the saved level in one line under its word, and offers the reset.
#[gpui::test]
fn the_effort_card_explains_the_saved_level(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let tree = composer_tree(
        cx,
        composer_person(Some("sonnet"), Some("high")),
        Some(effort_card_setup().as_str()),
    );
    assert!(tree.contains("Thinks it through."), "{tree}");
    assert!(tree.contains("composer-effort-reset"), "{tree}");
    assert!(tree.contains("Use Claude Code's own setting"), "{tree}");
}

/// With no level saved the card says Default and that Claude Code's own setting applies; no reset.
#[gpui::test]
fn the_effort_card_says_default_when_none_is_saved(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let tree = composer_tree(
        cx,
        composer_person(Some("sonnet"), None),
        Some(effort_card_setup().as_str()),
    );
    assert!(tree.contains("Default"), "{tree}");
    assert!(tree.contains("Claude Code's own setting."), "{tree}");
    assert!(!tree.contains("composer-effort-reset"), "{tree}");
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
    assert!(!tree.contains("lead-terminal"), "{tree}");
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

/// The startup reveal draws the note, and a lead that has not had its SessionStart cannot send.
fn startup_values(items: Value) -> BTreeMap<String, Value> {
    conversation_values_started(
        items,
        "shown",
        json!("startup"),
        Some("Claude is asking something before it starts."),
        false,
    )
}

#[gpui::test]
fn the_startup_reveal_draws_its_note(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe_settled(
        cx,
        &staged,
        "mods/composer",
        startup_values(json!([])),
        None,
    )
    .unwrap();
    assert!(tree.contains("composer-reveal-note"), "{tree}");
    assert!(
        tree.contains("Claude is asking something before it starts."),
        "{tree}"
    );
}

#[gpui::test]
fn a_lead_that_has_not_started_does_not_send(cx: &mut TestAppContext) {
    let effects = composer_live_run_after_clicks(
        cx,
        startup_values(json!([])),
        "this.text.set_value('hello');",
        &["composer-send"],
    );
    assert!(
        !effects
            .iter()
            .any(|args| args.first() == Some(&json!(["send", "t1", "hello"]))),
        "Send must no-op before SessionStart: {effects:?}"
    );
}

#[gpui::test]
fn a_started_lead_with_no_reveal_draws_the_conversation(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let items = json!([{"kind": "owner", "id": "o1", "text": "Hello"}]);
    let tree = describe_settled(
        cx,
        &staged,
        "mods/composer",
        conversation_values(items, "hidden", Value::Null, None),
        None,
    )
    .unwrap();
    assert!(tree.contains("composer-item-o1"), "{tree}");
    assert!(!tree.contains("composer-reveal-note"), "{tree}");
    assert!(!tree.contains("Claude is asking something before it starts."), "{tree}");
}

#[gpui::test]
fn a_missing_folder_asks_once_and_a_cancel_saves_nothing(cx: &mut TestAppContext) {
    let mut values = conversation_values(json!([]), "hidden", Value::Null, None);
    values.insert(
        "person".into(),
        json!({"lead": {"program": "claude", "model": "sonnet"}}),
    );
    let effects = composer_live_run_after_clicks(cx, values, "", &[]);
    let asked = effects
        .iter()
        .filter(|args| args.first() == Some(&json!("pickFolder")))
        .count();
    assert_eq!(asked, 1, "the folder is asked for once: {effects:?}");
    assert!(
        !effects
            .iter()
            .any(|args| args.first() == Some(&json!("lead.open"))),
        "a cancelled pick starts nothing: {effects:?}"
    );
    assert!(
        !effects.iter().any(|args| {
            args.first().and_then(Value::as_str) == Some("/fixture/app/settings.json")
        }),
        "a cancelled pick saves nothing: {effects:?}"
    );
}

/// A conversation taller than the pane keeps its newest item in view: the list
/// scrolls to its end (scroll_to_end) rather than leaving the newest item below the fold.
#[gpui::test]
fn a_long_conversation_stays_on_its_newest_item(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let items: Vec<Value> = (0..80)
        .map(|n| json!({"kind": "notice", "id": format!("n{n}"), "text": format!("Line {n}")}))
        .collect();
    let values = conversation_values(Value::Array(items), "hidden", Value::Null, None);
    let calls = Rc::new(RefCell::new(Calls::default()));
    let policy = Policy::new()
        .with_storage_path(staged.0.join("store-long-conversation.json"))
        .with_capabilities(Capabilities::new().storage(true).read_roots([staged.0.clone()]))
        .with_host_module(module(values, calls))
        .unwrap();
    gpui_shell::policy::set_default(policy);
    let runtime = ShellRuntime::new_isolated().unwrap();
    let root = staged.wrapper("mods/composer", "");
    let window = cx.add_window(|window, cx| Mounted(runtime.try_load(&root, window, cx).unwrap()));
    let mut visual = VisualTestContext::from_window(*window.deref(), cx);
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear(cx));
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear(cx));
    visual.run_until_parked();
    let list = visual
        .debug_bounds("composer-conversation")
        .expect("the conversation list is laid out");
    let newest = visual
        .debug_bounds("composer-item-n79")
        .expect("the newest item is laid out");
    let slack = gpui::px(1.0);
    assert!(
        newest.bottom() <= list.bottom() + slack && newest.bottom() > list.top(),
        "the newest item must be in view: item {newest:?}, list {list:?}"
    );
}

/// The box shares the conversation's column: at most 760 px, centred, and never closer than the 20 px gutter.
#[gpui::test]
fn the_box_and_the_conversation_share_one_centred_column(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let items = json!([{"kind": "notice", "id": "n0", "text": "Signed in."}]);
    let values = conversation_values(items, "hidden", Value::Null, None);
    let calls = Rc::new(RefCell::new(Calls::default()));
    let policy = Policy::new()
        .with_storage_path(staged.0.join("store-column.json"))
        .with_capabilities(Capabilities::new().storage(true).read_roots([staged.0.clone()]))
        .with_host_module(module(values, calls))
        .unwrap();
    gpui_shell::policy::set_default(policy);
    let runtime = ShellRuntime::new_isolated().unwrap();
    let root = staged.wrapper("mods/composer", "");
    let window = cx.add_window(|window, cx| Mounted(runtime.try_load(&root, window, cx).unwrap()));
    let mut visual = VisualTestContext::from_window(*window.deref(), cx);
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear(cx));
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear(cx));
    visual.run_until_parked();
    let list = visual
        .debug_bounds("composer-conversation")
        .expect("the conversation list is laid out");
    let box_ = visual.debug_bounds("composer-box").expect("the box is laid out");
    let slack = gpui::px(1.0);
    assert!(box_.size.width <= gpui::px(760.0) + slack, "the column is at most 760 px: {box_:?}");
    let left = box_.left() - list.left();
    let right = list.right() - box_.right();
    assert!((left - right).abs() <= slack, "the box is centred: left {left:?}, right {right:?}");
    assert!(left >= gpui::px(20.0) - slack, "a 20 px gutter at least: {left:?}");
}

/// The lead's values for the context ring, Claude's limits, the running model and compaction. Every field the
/// composer reads is here; `extra` overrides any of them.
fn usage_values(usage: Value, extra: Value) -> BTreeMap<String, Value> {
    let mut lead = json!({
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
    });
    lead.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    let mut values = conversation_values(json!([]), "hidden", Value::Null, None);
    values.insert("lead".into(), lead);
    values.insert("person".into(), composer_person(Some("opus"), Some("high")));
    values
}

fn usage_with(context_tokens: u64, five_hour: Value) -> Value {
    json!({
        "context_tokens": context_tokens,
        "context_window": 1000000,
        "compact_at": 444000,
        "cache_ttl": null,
        "cache_expires_at": null,
        "five_hour": five_hour,
        "seven_day": null,
        "session_id": null,
        "seen_ms": 0
    })
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn composer_draws(cx: &mut TestAppContext, values: BTreeMap<String, Value>, setup: Option<&str>) -> String {
    let staged = Staged::new();
    describe_settled(cx, &staged, "mods/composer", values, setup).unwrap()
}

/// The ring is drawn when the context is counted, with its share as the tooltip; not when usage is null.
#[gpui::test]
fn the_context_ring_draws_when_the_context_is_counted(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let tree = composer_draws(cx, usage_values(usage_with(300000, Value::Null), json!({})), None);
    assert!(tree.contains("composer-usage"), "{tree}");
    assert!(tree.contains("Context 68%"), "{tree}");

    let none = composer_draws(cx, usage_values(Value::Null, json!({})), None);
    assert!(!none.contains("composer-usage"), "{none}");
}

/// A present window at 75% or more says itself in the row; under 75 it says nothing.
#[gpui::test]
fn a_limit_says_itself_only_when_it_matters(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let soon = unix_now() + 3600;
    let high = composer_draws(
        cx,
        usage_values(
            usage_with(300000, json!({"used_percentage": 82, "resets_at": soon})),
            json!({}),
        ),
        None,
    );
    assert!(high.contains("composer-limit"), "{high}");
    assert!(high.contains("5h 82%"), "{high}");

    let low = composer_draws(
        cx,
        usage_values(
            usage_with(300000, json!({"used_percentage": 40, "resets_at": soon})),
            json!({}),
        ),
        None,
    );
    assert!(!low.contains("composer-limit"), "{low}");
}

/// The chip names the model the lead says it runs; the saved label stands until it has said.
#[gpui::test]
fn the_chip_names_the_running_model(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let tree = composer_draws(
        cx,
        usage_values(Value::Null, json!({"model": {"id": "claude-haiku-5-5", "name": "Haiku 5.5"}})),
        None,
    );
    assert!(tree.contains("Haiku 5.5"), "{tree}");
}

/// While Claude compacts the note says so, and Send neither sends nor clears the box.
#[gpui::test]
fn while_compacting_the_composer_holds_the_message(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let values = usage_values(usage_with(300000, Value::Null), json!({"compacting": true}));
    let tree = composer_draws(cx, values.clone(), None);
    assert!(tree.contains("Tidying up…"), "{tree}");

    let effects = composer_live_run_after_clicks(
        cx,
        usage_values(usage_with(300000, Value::Null), json!({"compacting": true})),
        "this.text.set_value('hello');",
        &["composer-send"],
    );
    assert!(
        !effects
            .iter()
            .any(|args| args.first().is_some_and(|first| first.to_string().contains("send"))),
        "Send must not reach kom send while compacting: {effects:?}"
    );
}

/// The ring opens a card with the context in K and the Compact now button, which is live when the lead waits.
#[gpui::test]
fn the_card_shows_the_context_and_compact_now(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let values = usage_values(usage_with(271000, Value::Null), json!({}));
    let tree = composer_draws(cx, values, Some("this.usageOpen = true;"));
    assert!(tree.contains("of 444K"), "{tree}");
    assert!(tree.contains("271K"), "{tree}");
    assert!(tree.contains("Compact now"), "{tree}");
    assert!(tree.contains("composer-compact-now"), "{tree}");
    assert!(tree.contains("Kompitum tidies up by itself while you're away."), "{tree}");
}

/// The composer reports a draft through the shared flag, set only when it changes.
#[gpui::test]
fn a_draft_sets_the_shared_flag(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let calls = Rc::new(RefCell::new(Calls::default()));
    let policy = Policy::new()
        .with_storage_path(staged.0.join("store-draft.json"))
        .with_capabilities(Capabilities::new().storage(true).read_roots([staged.0.clone()]))
        .with_host_module(module(usage_values(Value::Null, json!({})), calls.clone()))
        .unwrap();
    gpui_shell::policy::set_default(policy);
    let runtime = ShellRuntime::new_isolated().unwrap();
    let root = staged.wrapper("mods/composer", "this.text.set_value('hello'); this.syncDraft();");
    let window = cx.add_window(|window, cx| Mounted(runtime.try_load(&root, window, cx).unwrap()));
    let mut visual = VisualTestContext::from_window(*window.deref(), cx);
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear(cx));
    visual.run_until_parked();
    drop(visual);
    cx.run_until_parked();
    window.update(cx, |_, window, _| window.remove_window()).ok();
    cx.run_until_parked();
    let shared = calls.borrow().shared.get("lead.draft").cloned();
    assert_eq!(shared, Some(Value::Bool(true)), "a typed draft sets lead.draft: {shared:?}");
}
