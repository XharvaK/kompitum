//! Icon-only controls name themselves for a screen reader: the window's
//! buttons, a pane header's four buttons and the spaces list's "new".
#![allow(unused_imports)]

use super::*;

fn assert_label(tree: &str, id: &str, label: &str) {
    let line = tree
        .lines()
        .find(|line| line.contains(&format!(":id[Str(\"{id}\")]")))
        .unwrap_or_else(|| panic!("no {id}: {tree}"));
    assert!(
        line.contains(&format!(":accessibility_label[Str({label:?})]")),
        "{id} should announce {label:?}: {line}"
    );
}

#[gpui::test]
fn window_buttons_carry_their_labels(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let values = fixture(true, props(), updates("current", "current"));
    let tree = describe_settled(cx, &staged, "mods/topbar", values, None).unwrap();
    assert_label(&tree, "minimize", "Minimize");
    assert_label(&tree, "maximize", "Maximize");
    assert_label(&tree, "close-window", "Close");

    let values = fixture(true, props(), updates("current", "current"));
    let tree = describe_settled(cx, &staged, "mods/tabs", values, None).unwrap();
    assert_label(&tree, "minimize", "Minimize");
    assert_label(&tree, "maximize", "Maximize");
    assert_label(&tree, "close-window", "Close");

    let mut values = fixture(true, props(), updates("current", "current"));
    values.insert(
        "window".into(),
        json!({"platform":"linux","titlebar":"none","buttons":true,"maximized":true,"fullscreen":false,"active":true}),
    );
    let tree = describe_settled(cx, &staged, "mods/topbar", values, None).unwrap();
    assert_label(&tree, "maximize", "Restore");
}

#[gpui::test]
fn a_pane_headers_four_buttons_carry_their_labels(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let values = fixture(true, props(), updates("current", "current"));
    let tree = describe_with_values(cx, &staged, "mods/pane", values, Some("this.hovered = true;")).unwrap();
    assert_label(&tree, "ph-r-p1", "Split right");
    assert_label(&tree, "ph-d-p1", "Split down");
    assert_label(&tree, "ph-z-p1", "Zoom pane");
    assert_label(&tree, "ph-x-p1", "Close pane");
}

#[gpui::test]
fn the_spaces_list_new_button_carries_its_label(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe(
        cx,
        &staged,
        "mods/spaces",
        false,
        props(),
        updates("current", "current"),
        None,
    )
    .unwrap();
    assert_label(&tree, "space-new", "New space");
}
