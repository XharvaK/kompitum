//! Render tests for the rework (audit/REWORK-2026-11-09.md): see the parent module for the helpers.
#![allow(unused_imports)]

use super::*;

#[gpui::test]
fn machine_needing_an_answer_offers_a_terminal(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe_settled(
        cx,
        &staged,
        "mods/machine-add",
        fixture(false, props(), updates("current", "current")),
        Some("this.typed = 'host'; this.add('host', cx);"),
    )
    .unwrap();
    assert!(tree.contains("Finish in a terminal"), "{tree}");
    assert!(
        tree.contains(
            "host needs an answer from you: a password, a new host key, or installing Kompitum there."
        ),
        "{tree}"
    );
    assert!(
        tree.contains("kom machine add host"),
        "the button says what it runs: {tree}"
    );
}

#[gpui::test]
fn empty_typed_address_leaves_add_disabled(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe_settled(
        cx,
        &staged,
        "mods/machine-add",
        fixture(false, props(), updates("current", "current")),
        None,
    )
    .unwrap();
    let button = tree
        .lines()
        .find(|line| line.contains("ma-add-typed"))
        .unwrap_or_else(|| panic!("no typed Add button: {tree}"));
    assert!(
        button.contains(".opacity[Number(0.4)]"),
        "it is drawn disabled: {button}"
    );
    assert!(
        !button.contains(":on_click(fn)"),
        "an empty address does nothing: {button}"
    );
}
