//! Render tests for App NB-6: a CLI failure shows one meaningful line, and
//! keeps the rest in the tooltip (see the parent module for the helpers).
#![allow(unused_imports)]

use super::*;

const DUMP: &str = "could not lock the vault\nand a stack dump";
const FIRST: &str = "could not lock the vault";

fn assert_first_line_and_full_tooltip(tree: &str) {
    assert!(
        tree.contains(&format!(r#"text {FIRST:?}"#)),
        "the first meaningful line is drawn: {tree}"
    );
    assert!(
        !tree.contains(&format!(r#"text {DUMP:?}"#)),
        "the raw dump is not drawn as text: {tree}"
    );
    assert!(
        tree.contains(&format!(":tooltip[Str({DUMP:?})]")),
        "the tooltip keeps the whole text: {tree}"
    );
}

/// The mailbox's vault read failed: one line on the list, the rest on hover.
#[gpui::test]
fn a_mailbox_cli_failure_shows_the_first_line_and_keeps_the_rest_in_the_tooltip(
    cx: &mut TestAppContext,
) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut values = fixture(true, props(), updates("current", "current"));
    values.insert("vaultFail".into(), json!(DUMP));
    let tree = describe_settled(cx, &staged, "frame/mailbox", values, None).unwrap();
    assert_first_line_and_full_tooltip(&tree);
}

/// The Line's thread read failed the same way.
#[gpui::test]
fn a_line_cli_failure_shows_the_first_line_and_keeps_the_rest_in_the_tooltip(
    cx: &mut TestAppContext,
) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut values = fixture(true, props(), updates("current", "current"));
    values.insert("threadFail".into(), json!(DUMP));
    let tree = describe_settled(cx, &staged, "mods/line", values, None).unwrap();
    assert_first_line_and_full_tooltip(&tree);
}

/// Sending from the composer failed the same way.
#[gpui::test]
fn a_composer_cli_failure_shows_the_first_line_and_keeps_the_rest_in_the_tooltip(
    cx: &mut TestAppContext,
) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut values = fixture(true, props(), updates("current", "current"));
    values.insert("knownAgents".into(), agent_catalog());
    values.insert("sendFail".into(), json!(DUMP));
    let tree = describe_settled(
        cx,
        &staged,
        "mods/composer",
        values,
        Some("this.text.set_value('hello'); this.send(cx);"),
    )
    .unwrap();
    assert_first_line_and_full_tooltip(&tree);
}

/// Adding a machine failed the same way.
#[gpui::test]
fn a_machine_add_cli_failure_shows_the_first_line_and_keeps_the_rest_in_the_tooltip(
    cx: &mut TestAppContext,
) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut values = fixture(false, props(), updates("current", "current"));
    values.insert("machineAddFail".into(), json!(DUMP));
    let tree = describe_settled(
        cx,
        &staged,
        "mods/machine-add",
        values,
        Some("this.typed = 'box'; this.add('box', cx);"),
    )
    .unwrap();
    assert_first_line_and_full_tooltip(&tree);
}
