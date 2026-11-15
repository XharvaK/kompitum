//! Render tests for `text()`: a mod draws the interface word for a key.
#![allow(unused_imports)]

use super::*;

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
