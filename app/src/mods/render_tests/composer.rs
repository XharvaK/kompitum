//! Render tests for the composer's box (audit/REWORK-2026-11-09.md, C1 to C6): see the parent module for the
//! helpers.
#![allow(unused_imports)]

use super::*;

/// One box: no placeholder, "+" to attach, the switch to type in the terminal, the lead's model, round Send;
/// no plan ring and no mail button.
#[gpui::test]
fn the_composer_is_one_box_with_attach_model_and_send(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut values = fixture(true, props(), updates("current", "current"));
    values.insert("knownAgents".into(), agent_catalog());
    let tree = describe_settled(cx, &staged, "mods/composer", values, None).unwrap();
    for id in ["composer-box", "composer-attach", "composer-type", "composer-model", "composer-send"] {
        assert!(tree.contains(id), "{id}: {tree}");
    }
    for gone in ["Message the lead", "composer-plan", "composer-mail"] {
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
