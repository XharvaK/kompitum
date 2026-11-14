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
    let theme = fixture(false, props(), updates("current", "current"))["theme"].clone();
    let finish = tree
        .lines()
        .find(|line| line.contains("ma-finish-host"))
        .unwrap_or_else(|| panic!("no Finish button: {tree}"));
    assert!(finish.contains(theme["accent_fill"].as_str().unwrap()), "{finish}");
    assert!(finish.contains(theme["accent_foreground"].as_str().unwrap()), "{finish}");
    assert!(
        !finish.contains(&format!(".bg[Str(\"{}\")]", theme["solid"].as_str().unwrap())),
        "{finish}"
    );
}

#[gpui::test]
fn a_typed_address_uses_the_primary_look(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe_settled(
        cx,
        &staged,
        "mods/machine-add",
        fixture(false, props(), updates("current", "current")),
        Some("this.typed = 'host';"),
    )
    .unwrap();
    let theme = fixture(false, props(), updates("current", "current"))["theme"].clone();
    let button = tree
        .lines()
        .find(|line| line.contains("ma-add-typed"))
        .unwrap_or_else(|| panic!("no Add button: {tree}"));
    assert!(button.contains(theme["accent_fill"].as_str().unwrap()), "{button}");
    assert!(button.contains(theme["accent_foreground"].as_str().unwrap()), "{button}");
    assert!(
        !button.contains(&format!(".bg[Str(\"{}\")]", theme["solid"].as_str().unwrap())),
        "{button}"
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
    // Disabled, the UD way: the fill goes and an outline keeps the shape (never a faded fill).
    assert!(
        button.contains(".border_1") && !button.contains(".bg[") && !button.contains(".opacity"),
        "it is drawn disabled: {button}"
    );
    assert!(
        !button.contains(":on_click(fn)"),
        "an empty address does nothing: {button}"
    );
}
