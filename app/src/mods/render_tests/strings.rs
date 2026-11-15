//! Render tests for `text()`: a mod draws the interface word for a key.
#![allow(unused_imports)]

use super::*;

/// The turn summary and the mods' fallbacks are words from the table, not English in code.
#[test]
fn the_new_words_are_in_the_shipped_table() {
    let table: BTreeMap<String, String> =
        serde_json::from_str(include_str!("../../../defaults/strings/en.json")).unwrap();
    for (key, value) in [
        ("composer.files-changed-one", "{count} file changed"),
        ("composer.files-changed", "{count} files changed"),
        ("bell.any-agent", "An agent"),
        ("board.kom-exited", "kom exited {code}"),
        ("limits.kom-exited", "kom exited {code}"),
        ("line.kom-exited", "kom exited {code}"),
        ("machine-add.no-answer", "it didn't answer"),
        ("workspace-chrome.exit-code", "exit {code}"),
        ("tabs.close-tab", "Close tab"),
        ("tabs.new", "New"),
    ] {
        assert_eq!(table.get(key).map(String::as_str), Some(value), "{key}");
    }
}

/// The shipped table is empty, so a key it doesn't have comes back as written:
/// the same value the real function returns, through the module a mod imports.
#[gpui::test]
fn a_mod_draws_a_key_english_lacks_as_itself(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let dir = staged.0.join("mods/strings-test");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("main.js"),
        r#"
import { View, div } from "gpui-kit";
import { text } from "kompitum";
export default class Missing extends View {
  render() {
    return div().child(text("strings-test.missing", { n: 2 }));
  }
}"#,
    )
    .unwrap();
    let tree = describe_settled(
        cx,
        &staged,
        "mods/strings-test",
        fixture(false, json!({}), updates("current", "current")),
        None,
    )
    .unwrap();
    assert!(tree.contains("strings-test.missing"), "{tree}");
}
