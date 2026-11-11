//! Render tests for the palette, the shortcuts sheet, the keys and What's new (the UX audit of 2026-11-11): see the parent module for the helpers.
#![allow(unused_imports)]

use super::*;

// The desk layout is the default while no layout is shown yet, so these run on desk.

#[gpui::test]
fn palette_on_desk_groups_terminals_and_drops_tab_words(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe(
        cx,
        &staged,
        "frame/palette",
        true,
        props(),
        updates("current", "current"),
        None,
    )
    .unwrap();
    assert!(
        tree.contains("Terminals") && tree.contains("New terminal"),
        "{tree}"
    );
    assert!(!tree.contains("New tab"), "{tree}");
}

#[gpui::test]
fn shortcuts_sheet_on_desk_draws_each_command_once(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe(
        cx,
        &staged,
        "mods/help",
        true,
        props(),
        updates("current", "current"),
        None,
    )
    .unwrap();
    assert_eq!(tree.matches("Next terminal").count(), 1, "{tree}");
    assert!(tree.contains("Terminals") && tree.contains("More"), "{tree}");
}

#[gpui::test]
fn whats_new_draws_inline_code_without_backticks(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let notes = json!([{
        "version": "0.3.0", "date": "2026-11-11",
        "items": [{"headline": "Its own core, `kom`.", "detail": "Run `kom --help` for the commands.", "fixed": false, "media": []}],
        "spotlight": true
    }]);
    let mut p = props();
    p["notes"] = notes;
    let tree = describe(
        cx,
        &staged,
        "frame/whatsnew",
        true,
        p,
        updates("current", "current"),
        None,
    )
    .unwrap();
    assert!(
        tree.contains("Its own core, kom.") && tree.contains("Run kom --help for the commands."),
        "{tree}"
    );
    assert!(!tree.contains('`'), "{tree}");
}
