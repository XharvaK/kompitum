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
    assert!(tree.contains("The lead's model and reasoning"), "{tree}");
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

/// Picking a model keeps the menu open: the reasoning column stays beside the models (there is no second step).
#[gpui::test]
fn picking_a_model_keeps_the_models_view(cx: &mut TestAppContext) {
    let found = composer_laid_out_after_clicks(
        cx,
        composer_person(Some("sonnet"), None),
        "this.modelOpen = true;",
        &["composer-model-opus"],
        &["composer-menu-divider", "composer-model-opus"],
    );
    assert!(found[0].1, "the reasoning column stays beside the models: {found:?}");
    assert!(found[1].1, "the models stay: {found:?}");
}

/// The menu holds the models and the reasoning levels side by side, with Default first and no effort card.
#[gpui::test]
fn the_model_menu_has_two_columns(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let tree = composer_tree(
        cx,
        composer_person(Some("sonnet"), Some("low")),
        Some("this.modelOpen = true;"),
    );
    for id in ["composer-model-opus", "composer-menu-divider", "composer-level-default", "composer-level-high"] {
        assert!(tree.contains(id), "{id}: {tree}");
    }
    assert!(tree.contains("Reasoning"), "{tree}");
    assert!(!tree.contains("composer-effort-slider"), "{tree}");
    assert!(!tree.contains("composer-reasoning-row"), "{tree}");
}

/// A level row saves that level and the menu stays open; the save writes the lead's effort to settings.
#[gpui::test]
fn picking_a_level_saves_it_and_keeps_the_menu_open(cx: &mut TestAppContext) {
    let found = composer_laid_out_after_clicks(
        cx,
        composer_person(Some("sonnet"), Some("low")),
        "this.modelOpen = true;",
        &["composer-level-high"],
        &["composer-menu-divider", "composer-level-high", "composer-model-sonnet"],
    );
    assert!(found.iter().all(|(_, shown)| *shown), "the menu stays open: {found:?}");
    let mut values = usage_values(Value::Null, json!({}));
    values.insert("person".into(), saved_lead_with_folder("sonnet", "low"));
    let effects = composer_live_run_after_clicks(cx, values, "this.modelOpen = true;", &["composer-level-high"]);
    assert!(
        effects.iter().any(|args| args
            .iter()
            .any(|arg| arg.as_str().is_some_and(|text| text.contains("\"effort\": \"high\"")))),
        "the high level is written to settings: {effects:?}"
    );
}

/// A helper's shared values for the fixture: `lead.running` as the app sets it at each start.
fn with_shared(mut values: BTreeMap<String, Value>, shared: Value) -> BTreeMap<String, Value> {
    values.insert("sharedValues".into(), shared);
    values
}

fn running_lead(terminal: &str, model: &str, effort: &str) -> Value {
    json!({ "lead.running": { "program": "claude", "terminal": terminal, "model": model, "effort": effort } })
}

/// The saved lead for the live tests: a folder is set, so the composer doesn't ask for one.
fn saved_lead_with_folder(model: &str, effort: &str) -> Value {
    json!({"lead": {"program": "claude", "model": model, "effort": effort, "folder": "C:/work"}})
}

/// The chip shows the reasoning the running lead was started with, and a dot while the saved level waits.
#[gpui::test]
fn the_chip_shows_what_the_lead_was_started_with(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let values = with_shared(usage_values(Value::Null, json!({})), running_lead("t1", "opus", "low"));
    // The closed menu's content is in the dump too, and its Reasoning row shows the saved "High": "Low" is the chip's.
    let tree = composer_draws(cx, values, None);
    assert!(tree.contains("Low"), "{tree}");
    assert!(tree.contains("composer-model-pending"), "{tree}");
    assert!(tree.contains("Changes wait for the lead's restart"), "{tree}");
}

#[gpui::test]
fn no_pending_dot_when_the_saved_settings_run(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let values = with_shared(usage_values(Value::Null, json!({})), running_lead("t1", "opus", "high"));
    let tree = composer_draws(cx, values, None);
    assert!(tree.contains("High"), "{tree}");
    assert!(!tree.contains("composer-model-pending"), "{tree}");
    assert!(tree.contains("The lead's model and reasoning"), "{tree}");
}

/// Another terminal's lead start says nothing about this composer's lead.
#[gpui::test]
fn another_terminals_start_is_ignored(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let values = with_shared(usage_values(Value::Null, json!({})), running_lead("t9", "opus", "low"));
    let tree = composer_draws(cx, values, None);
    assert!(tree.contains("High"), "{tree}");
    assert!(!tree.contains("composer-model-pending"), "{tree}");
}

/// The dump line of the Restart now button, which says `on_click` only when the button is live.
fn restart_button_line(tree: &str) -> &str {
    tree.lines().find(|line| line.contains("Str(\"composer-restart\")")).unwrap_or_default()
}

/// Restart now is drawn under the models view while a change waits, live while the lead waits, and runs
/// lead.restart for this lead. The test window clips the strip's row, so the click is not used: the setup calls
/// the same method the click calls.
#[gpui::test]
fn restart_now_runs_lead_restart(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let mut values = with_shared(usage_values(Value::Null, json!({})), running_lead("t1", "opus", "low"));
    values.insert("person".into(), saved_lead_with_folder("opus", "high"));
    let tree = composer_draws(cx, values.clone(), Some("this.modelOpen = true;"));
    assert!(restart_button_line(&tree).contains("on_click"), "{tree}");
    let effects = composer_live_run_after_clicks(cx, values, "this.modelOpen = true; this.restartNow(cx);", &[]);
    assert!(
        effects
            .iter()
            .any(|args| args.first() == Some(&json!("lead.restart")) && args.get(1) == Some(&json!({"terminal": "t1"}))),
        "{effects:?}"
    );
}

/// While the lead works the button is drawn dim with its busy tooltip, and has no click.
#[gpui::test]
fn restart_now_waits_while_the_lead_works(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let mut values = with_shared(
        usage_values(Value::Null, json!({"state": "working"})),
        running_lead("t1", "opus", "low"),
    );
    values.insert("person".into(), saved_lead_with_folder("opus", "high"));
    let tree = composer_draws(cx, values, Some("this.modelOpen = true;"));
    assert!(!restart_button_line(&tree).contains("on_click"), "{tree}");
    assert!(tree.contains("The lead is busy. Restart when it's done."), "{tree}");
}

/// A working lead with no word for ten minutes reads "Working?"; the tooltip says why.
#[gpui::test]
fn a_quiet_working_lead_reads_working_with_a_question_mark(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let tree = composer_draws(
        cx,
        usage_values(Value::Null, json!({"state": "working", "heard_ms": 1})),
        None,
    );
    assert!(tree.contains("Working?"), "{tree}");
    assert!(tree.contains("No word from the lead for 10 minutes."), "{tree}");
}

#[gpui::test]
fn a_fresh_working_lead_reads_working(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let now_ms = unix_now() * 1000;
    let tree = composer_draws(
        cx,
        usage_values(Value::Null, json!({"state": "working", "heard_ms": now_ms})),
        None,
    );
    assert!(tree.contains("Working"), "{tree}");
    assert!(!tree.contains("Working?"), "{tree}");
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
                || name == "openOverlay"
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
    // The empty conversation is the start view (the catalog name it showed is gone); no scroll column.
    assert!(tree.contains("composer-start"), "{tree}");
    assert!(!tree.contains("composer-conversation-empty"), "{tree}");
    assert!(tree.contains("Claude in your home folder"), "{tree}");
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
    // One row, named by leadName ("Claude" for the claude key), with no reveal note and no box under it.
    assert!(tree.contains("composer-starting"), "{tree}");
    assert!(
        tree.contains("Claude is asking something before it starts. Answer it above."),
        "{tree}"
    );
    assert!(!tree.contains("composer-reveal-note"), "{tree}");
    assert!(!tree.contains("composer-box"), "{tree}");
}

#[gpui::test]
fn a_lead_that_has_not_started_does_not_send(cx: &mut TestAppContext) {
    // Before SessionStart the box is not drawn at all, so there is no Send to press.
    let staged = Staged::new();
    let tree = describe_settled(
        cx,
        &staged,
        "mods/composer",
        startup_values(json!([])),
        Some("this.text.set_value('hello');"),
    )
    .unwrap();
    assert!(!tree.contains("composer-send"), "{tree}");
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
    assert!(tree.contains("Context 68% · Limits"), "{tree}");

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

/// The ring opens the Limits sheet for this lead's terminal, so the sheet can offer Compact now.
#[gpui::test]
fn the_ring_opens_the_limits_sheet(cx: &mut TestAppContext) {
    let effects = composer_live_run_after_clicks(
        cx,
        usage_values(usage_with(300000, Value::Null), json!({})),
        "",
        &["composer-usage"],
    );
    assert!(
        effects.iter().any(|args| args.first().is_some_and(|spec| {
            spec.get("mod") == Some(&json!("limits")) && spec.pointer("/props/terminal") == Some(&json!("t1"))
        })),
        "the ring opens the limits sheet with the terminal: {effects:?}"
    );
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

/// Enter does nothing on a card that has just shown: a key pressed in passing cannot answer it.
#[gpui::test]
fn enter_does_nothing_right_after_a_card_arrives(cx: &mut TestAppContext) {
    let answered = composer_run_effects_after_keys(
        cx,
        composer_with_ask("permission", json!({"tool_name": "Bash", "tool_input": {"command": "ls"}})),
        "enter",
        // The card "arrived" an hour from now, so the wait can't run out on a slow machine.
        Some("this.askId = 'a1'; this.askSince = Date.now() + 3600000;"),
    );
    assert!(
        !answered.iter().any(|args| args.first() == Some(&json!("lead.answer"))),
        "Enter must wait for the card to settle: {answered:?}"
    );
}

/// A draft in the box holds the card's Enter: the words stay, and the card says the draft is kept.
#[gpui::test]
fn enter_does_nothing_while_a_draft_waits(cx: &mut TestAppContext) {
    let setup = "this.askId = 'a1'; this.askSince = 0; this.text.set_value('half a thought');";
    let answered = composer_run_effects_after_keys(
        cx,
        composer_with_ask("permission", json!({"tool_name": "Bash", "tool_input": {"command": "ls"}})),
        "enter",
        Some(setup),
    );
    assert!(
        !answered.iter().any(|args| args.first() == Some(&json!("lead.answer"))),
        "Enter must not answer over a draft: {answered:?}"
    );
    let staged = Staged::new();
    let tree = describe_settled(
        cx,
        &staged,
        "mods/composer",
        composer_with_ask("permission", json!({"tool_name": "Bash", "tool_input": {"command": "ls"}})),
        Some(setup),
    )
    .unwrap();
    assert!(tree.contains("composer-draft-kept"), "{tree}");
    assert!(tree.contains("You were typing. Click Allow"), "{tree}");
}

/// Escape acts at once, even on a card that has just shown: it is never held back.
#[gpui::test]
fn escape_still_denies_at_once(cx: &mut TestAppContext) {
    let answered = composer_run_effects_after_keys(
        cx,
        composer_with_ask("permission", json!({"tool_name": "Bash", "tool_input": {"command": "ls"}})),
        "escape",
        None,
    );
    assert!(
        answered.iter().any(|args| {
            args.first() == Some(&json!("lead.answer"))
                && args.get(1) == Some(&json!({"id": "a1", "answer": {"action": "deny"}}))
        }),
        "Escape must deny at once: {answered:?}"
    );
}

/// With no allow-always offered, Deny sits right after Allow rather than at the far end of the row.
#[gpui::test]
fn deny_sits_beside_allow(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let calls = Rc::new(RefCell::new(Calls::default()));
    let values = composer_with_ask("permission", json!({"tool_name": "Bash", "tool_input": {"command": "ls"}}));
    let policy = Policy::new()
        .with_storage_path(staged.0.join("store-deny.json"))
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
    let allow = visual.debug_bounds("composer-allow").expect("Allow is laid out");
    let deny = visual.debug_bounds("composer-deny").expect("Deny is laid out");
    drop(visual);
    cx.run_until_parked();
    window.update(cx, |_, window, _| window.remove_window()).ok();
    cx.run_until_parked();
    let gap = deny.left() - allow.right();
    assert!(gap < gpui::px(120.0), "Deny must sit beside Allow: allow {allow:?}, deny {deny:?}");
}

/// With a card open the box grows, and the list above it must still end on the newest item.
#[gpui::test]
fn a_card_keeps_the_newest_item_in_view(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let items: Vec<Value> = (0..80)
        .map(|n| json!({"kind": "notice", "id": format!("n{n}"), "text": format!("Line {n}")}))
        .collect();
    let mut values = conversation_values(Value::Array(items), "hidden", Value::Null, None);
    values.insert(
        "lead".into(),
        json!({
            "state": "asking",
            "ask": {"id": "a1", "terminal": "t1", "kind": "permission", "input": {"tool_name": "Bash", "tool_input": {"command": "ls"}}},
            "started": true,
            "terminal": "hidden",
            "reveal": null,
            "reveal_why": null,
            "conversation": { "version": 1, "length": 80 }
        }),
    );
    let calls = Rc::new(RefCell::new(Calls::default()));
    let policy = Policy::new()
        .with_storage_path(staged.0.join("store-card-view.json"))
        .with_capabilities(Capabilities::new().storage(true).read_roots([staged.0.clone()]))
        .with_host_module(module(values, calls))
        .unwrap();
    gpui_shell::policy::set_default(policy);
    let runtime = ShellRuntime::new_isolated().unwrap();
    let root = staged.wrapper("mods/composer", "");
    let window = cx.add_window(|window, cx| Mounted(runtime.try_load(&root, window, cx).unwrap()));
    let mut visual = VisualTestContext::from_window(*window.deref(), cx);
    for _ in 0..3 {
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        visual.run_until_parked();
    }
    let list = visual
        .debug_bounds("composer-conversation")
        .expect("the conversation list is laid out");
    let newest = visual
        .debug_bounds("composer-item-n79")
        .expect("the newest item is laid out");
    let slack = gpui::px(1.0);
    assert!(
        newest.bottom() <= list.bottom() + slack && newest.bottom() > list.top(),
        "the newest item must stay in view under a card: item {newest:?}, list {list:?}"
    );
}

/// A tool row draws its command in the code font, and no backtick reaches the tree.
#[gpui::test]
fn a_tool_row_draws_its_command_without_backticks(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let items = json!([
        {"kind": "tool", "id": "u1", "name": "Bash", "summary": "Ran `git --version`", "state": "done"}
    ]);
    let tree = describe_settled(
        cx,
        &staged,
        "mods/composer",
        conversation_values(items, "hidden", Value::Null, None),
        None,
    )
    .unwrap();
    assert!(tree.contains("git --version"), "{tree}");
    assert!(!tree.contains('`'), "no backtick may reach the row: {tree}");
}

/// The chip's remove target is a 28 px hit area, though the × it holds is drawn at its old 14 px.
#[gpui::test]
fn a_chip_remove_target_is_28_px_wide(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let calls = Rc::new(RefCell::new(Calls::default()));
    let policy = Policy::new()
        .with_storage_path(staged.0.join("store-chip.json"))
        .with_capabilities(Capabilities::new().storage(true).read_roots([staged.0.clone()]))
        .with_host_module(module(usage_values(Value::Null, json!({})), calls))
        .unwrap();
    gpui_shell::policy::set_default(policy);
    let runtime = ShellRuntime::new_isolated().unwrap();
    let root = staged.wrapper("mods/composer", "this.addChip('/fixture/notes.txt');");
    let window = cx.add_window(|window, cx| Mounted(runtime.try_load(&root, window, cx).unwrap()));
    let mut visual = VisualTestContext::from_window(*window.deref(), cx);
    for _ in 0..2 {
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        visual.run_until_parked();
    }
    let remove = visual
        .debug_bounds("composer-chip-remove-0")
        .expect("the chip's remove target is laid out");
    let chip = visual
        .debug_bounds("composer-chip-0")
        .expect("the chip is laid out");
    assert_eq!(remove.size.width, gpui::px(28.0), "the hit area is 28 px wide: {remove:?}");
    assert_eq!(remove.size.height, gpui::px(28.0), "the hit area is 28 px tall: {remove:?}");
    assert_eq!(chip.size.height, gpui::px(22.0), "the chip keeps its 22 px height: {chip:?}");
}

/// Mounts the composer with `values`, runs `setup`, draws it settled, and returns the window's width and height
/// with the laid-out bounds of each id in `ids` (None when it isn't laid out).
fn composer_layout(
    cx: &mut TestAppContext,
    values: BTreeMap<String, Value>,
    setup: &str,
    ids: &[&'static str],
) -> (f32, f32, Vec<Option<gpui::Bounds<gpui::Pixels>>>) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let calls = Rc::new(RefCell::new(Calls::default()));
    let policy = Policy::new()
        .with_storage_path(staged.0.join("store-layout.json"))
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
    visual.update(|window, cx| window.draw(cx).clear(cx));
    visual.run_until_parked();
    let (width, height) = visual.update(|window, _| {
        let size = window.viewport_size();
        (f32::from(size.width), f32::from(size.height))
    });
    let found = ids.iter().map(|id| visual.debug_bounds(id)).collect();
    drop(visual);
    cx.run_until_parked();
    window.update(cx, |_, window, _| window.remove_window()).ok();
    cx.run_until_parked();
    let calls = calls.borrow();
    assert!(calls.unexpected.is_empty(), "{:?}", calls.unexpected);
    (width, height, found)
}

/// A new conversation: the headline and the box sit a little above the middle, the box's top below a quarter of
/// the window and its bottom above 85% of it.
#[gpui::test]
fn a_new_conversation_starts_in_the_middle(cx: &mut TestAppContext) {
    let values = conversation_values(json!([]), "hidden", Value::Null, None);
    let (_, height, found) = composer_layout(cx, values, "", &["composer-box"]);
    let tree = composer_draws(cx, conversation_values(json!([]), "hidden", Value::Null, None), None);
    assert!(tree.contains("composer-start"), "{tree}");
    assert!(tree.contains("What's next?"), "{tree}");
    let boxed = found[0].expect("the box is laid out");
    assert!(f32::from(boxed.top()) > height / 4.0, "the box starts below a quarter of {height}: {boxed:?}");
    assert!(f32::from(boxed.bottom()) < height * 0.85, "the box ends above 85% of {height}: {boxed:?}");
}

/// The start names the lead and the last part of its saved folder.
#[gpui::test]
fn the_start_names_the_folder(cx: &mut TestAppContext) {
    let mut values = conversation_values(json!([]), "hidden", Value::Null, None);
    values.insert(
        "person".into(),
        json!({"lead": {"program": "claude", "model": "opus", "folder": "C:\\Users\\x\\Projects\\kompitum"}}),
    );
    let tree = composer_draws(cx, values, None);
    assert!(tree.contains("Claude in kompitum"), "{tree}");
}

/// The first item moves the box to the bottom: no start view is drawn once there is an item.
#[gpui::test]
fn the_first_message_moves_the_box_to_the_bottom(cx: &mut TestAppContext) {
    let items = json!([{"kind": "owner", "id": "o1", "text": "Hello"}]);
    let tree = composer_draws(cx, conversation_values(items, "hidden", Value::Null, None), None);
    assert!(tree.contains("composer-item-o1"), "{tree}");
    assert!(!tree.contains("composer-start"), "{tree}");
    assert!(!tree.contains("What's next?"), "{tree}");
}

/// The model menu opens inside the window: its left edge at 0 or more, its right edge within the window's width.
#[gpui::test]
fn the_model_menu_opens_inside_the_window(cx: &mut TestAppContext) {
    let values = usage_values(Value::Null, json!({}));
    let (width, _, found) = composer_layout(cx, values, "this.modelOpen = true;", &["composer-menu"]);
    let menu = found[0].expect("the menu is laid out");
    assert!(f32::from(menu.left()) >= 0.0, "the menu's left edge is inside the window: {menu:?}");
    assert!(f32::from(menu.right()) <= width, "the menu's right edge is inside the {width}-wide window: {menu:?}");
}

/// The dump line of the mode word: it names its id, and says `on_click` only while the word is live.
fn mode_line(tree: &str) -> &str {
    tree.lines().find(|line| line.contains("Str(\"composer-mode\")")).unwrap_or_default()
}

/// The text colour the dump gives a line (its first `text_color`).
fn text_color_of(line: &str) -> &str {
    line.split("text_color[").nth(1).and_then(|rest| rest.split(']').next()).unwrap_or_default()
}

/// A lead whose hooks named `mode`, with the mode's guess flag; `extra` adds to the lead.
fn with_permission_mode(mode: &str, guessed: bool) -> BTreeMap<String, Value> {
    usage_values(Value::Null, json!({"permission_mode": mode, "mode_guessed": guessed}))
}

/// A lead with a permission card open: the box can't take a click on the mode word.
fn with_open_card(mode: &str) -> BTreeMap<String, Value> {
    usage_values(
        Value::Null,
        json!({
            "permission_mode": mode,
            "ask": {"id": "a1", "terminal": "t1", "kind": "permission", "input": {"tool_name": "Bash", "tool_input": {"command": "ls"}}}
        }),
    )
}

/// The box's bottom row: attach, then the model, the mode word, and Send at the far right.
#[gpui::test]
fn the_row_reads_model_level_and_mode(cx: &mut TestAppContext) {
    let tree = composer_draws(cx, with_permission_mode("default", false), None);
    assert!(tree.contains("Ask first"), "{tree}");
    assert!(!mode_line(&tree).is_empty(), "{tree}");
    let ids = ["composer-attach", "composer-model", "composer-mode", "composer-send"];
    let (_, _, found) = composer_layout(cx, with_permission_mode("default", false), "", &ids);
    let [attach, model, mode, send] = [0, 1, 2, 3].map(|i| found[i].unwrap_or_else(|| panic!("{} is laid out", ids[i])));
    assert!(f32::from(attach.left()) < f32::from(model.left()), "attach, then model: {attach:?} {model:?}");
    assert!(f32::from(model.left()) < f32::from(mode.left()), "model, then mode: {model:?} {mode:?}");
    assert!(f32::from(mode.left()) < f32::from(send.left()), "send is the rightmost: {mode:?} {send:?}");
}

/// Before Claude has reported a mode there is no mode word: no hook has named one.
#[gpui::test]
fn no_mode_word_before_claude_reports_one(cx: &mut TestAppContext) {
    let tree = composer_draws(cx, usage_values(Value::Null, json!({})), None);
    assert!(mode_line(&tree).is_empty(), "{tree}");
}

/// A click on the mode word runs lead.mode for this terminal.
#[gpui::test]
fn clicking_the_mode_runs_lead_mode(cx: &mut TestAppContext) {
    let effects = composer_live_run_after_clicks(cx, with_permission_mode("default", false), "", &["composer-mode"]);
    assert!(
        effects
            .iter()
            .any(|args| args.first() == Some(&json!("lead.mode")) && args.get(1) == Some(&json!({"terminal": "t1"}))),
        "{effects:?}"
    );
}

/// While a card is open the mode word is drawn but has no click: Claude is asking, so the mode can't switch.
#[gpui::test]
fn the_mode_word_waits_while_a_card_is_open(cx: &mut TestAppContext) {
    let tree = composer_draws(cx, with_open_card("default"), None);
    assert!(tree.contains("Ask first"), "{tree}");
    let line = mode_line(&tree);
    assert!(!line.is_empty(), "{tree}");
    assert!(!line.contains("on_click"), "{tree}");
}

/// A guessed mode is drawn dim, the colour a not-live mode word has; a confirmed live one is drawn muted.
#[gpui::test]
fn a_guessed_mode_is_drawn_dim(cx: &mut TestAppContext) {
    let guessed = composer_draws(cx, with_permission_mode("plan", true), None);
    let confirmed = composer_draws(cx, with_permission_mode("plan", false), None);
    let asking = composer_draws(cx, with_open_card("plan"), None);
    let dim = text_color_of(mode_line(&asking));
    assert!(!dim.is_empty(), "{asking}");
    assert_eq!(text_color_of(mode_line(&guessed)), dim, "a guessed mode is dim: {guessed}");
    assert_ne!(text_color_of(mode_line(&confirmed)), dim, "a confirmed live mode is not dim: {confirmed}");
}

/// "Compact first" shows only from 222K tokens, with a draft waiting: not below 222K, and not without a draft.
#[gpui::test]
fn compact_first_shows_only_from_222k_with_a_draft(cx: &mut TestAppContext) {
    let draft = "this.text.set_value('hello');";
    let below = composer_draws(cx, usage_values(usage_with(221999, Value::Null), json!({})), Some(draft));
    assert!(!below.contains("composer-compact-first"), "{below}");
    let no_draft = composer_draws(cx, usage_values(usage_with(300000, Value::Null), json!({})), None);
    assert!(!no_draft.contains("composer-compact-first"), "{no_draft}");
    let at = composer_draws(cx, usage_values(usage_with(222000, Value::Null), json!({})), Some(draft));
    assert!(at.contains("composer-compact-first"), "{at}");
}

/// With "Compact first" on, Send runs lead.compact and sends nothing; the text stays in the box while it waits.
#[gpui::test]
fn compact_first_then_send_compacts_and_keeps_the_text(cx: &mut TestAppContext) {
    let effects = composer_live_run_after_clicks(
        cx,
        usage_values(usage_with(300000, Value::Null), json!({})),
        "this.text.set_value('hello'); this.compactFirst = true;",
        &["composer-send"],
    );
    assert!(
        effects.iter().any(|args| args.first() == Some(&json!("lead.compact"))
            && args.get(1) == Some(&json!({"terminal": "t1"}))),
        "{effects:?}"
    );
    assert!(
        !effects
            .iter()
            .any(|args| args.first().and_then(Value::as_array).and_then(|words| words.first()) == Some(&json!("send"))),
        "nothing is sent while the lead tidies up: {effects:?}"
    );
    let waiting = composer_draws(
        cx,
        usage_values(usage_with(300000, Value::Null), json!({})),
        Some("this.text.set_value('hello'); this.compactThenSend = { at: Date.now(), started: false };"),
    );
    assert!(waiting.contains("Tidying up, then sending…"), "{waiting}");
    // The box's text is not in the render dump, so "the text stays" is not asserted here (see the report's Gaps).
}
