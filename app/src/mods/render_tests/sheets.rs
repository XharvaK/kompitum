//! The one sheet family (audit F15): the shared frame, and every sheet on it.

use super::*;

/// A mod whose whole body says which width it was given, so the frame's own
/// header and close are what the tree shows.
const SHEET_DEMO: &str = r#"
import { View, div } from "gpui-kit";
import { sheet } from "kompitum";
export default class SheetDemo extends View {
  init() { this.width = 880; }
  render() {
    return sheet(
      { title: "Sheet demo", width: this.width, onClose: () => {}, footer: div().child("Footer words") },
      div().child("Body words"),
    );
  }
}
"#;

#[gpui::test]
fn the_sheet_frame_has_a_title_close_body_and_footer(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let dir = staged.0.join("mods/sheet-demo");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("mod.json"), r#"{ "place": "none" }"#).unwrap();
    std::fs::write(dir.join("main.js"), SHEET_DEMO).unwrap();
    for (width, setup) in [(880, None), (640, Some("this.width = 640;"))] {
        let tree = describe_with_values(
            cx,
            &staged,
            "mods/sheet-demo",
            fixture(false, props(), updates("current", "current")),
            setup,
        )
        .unwrap();
        assert!(tree.contains("Sheet demo"), "{tree}");
        assert!(tree.contains("Body words"), "{tree}");
        assert!(tree.contains("Footer words"), "{tree}");
        assert!(
            tree.contains(".h[Number(60.0)]"),
            "the footer is 60 high: {tree}"
        );
        assert!(tree.contains("sheet-close"), "the frame draws the close: {tree}");
        assert!(
            tree.contains("Close"),
            "its tooltip and accessibility label say Close: {tree}"
        );
        assert!(
            tree.contains(&format!(".w[Number({width}.0)]")),
            "the width it was given: {tree}"
        );
    }
}

/// The shared frame's only word is the table's: a sheet drawn through
/// `kompitum` carries it as the close button's tooltip and accessibility label.
#[gpui::test]
fn a_sheet_drawn_through_kompitum_carries_the_tables_close_label(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let dir = staged.0.join("mods/sheet-demo");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("mod.json"), r#"{ "place": "none" }"#).unwrap();
    std::fs::write(dir.join("main.js"), SHEET_DEMO).unwrap();
    let tree = describe_with_values(
        cx,
        &staged,
        "mods/sheet-demo",
        fixture(false, props(), updates("current", "current")),
        None,
    )
    .unwrap();
    let table: BTreeMap<String, String> =
        serde_json::from_str(include_str!("../../../defaults/strings/en.json")).unwrap();
    let label = table.get("sheet.close").expect("sheet.close is in the table");
    assert_eq!(label, "Close");
    assert!(
        tree.contains(&format!(":tooltip[Str({label:?})]")),
        "the close tooltip is the table's label: {tree}"
    );
    assert!(
        tree.contains(&format!(":accessibility_label[Str({label:?})]")),
        "the close label is the table's label: {tree}"
    );
}

#[gpui::test]
fn settings_is_a_sheet(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe(
        cx,
        &staged,
        "frame/settings",
        false,
        props(),
        updates("current", "current"),
        None,
    )
    .unwrap();
    assert!(tree.contains("Settings"), "{tree}");
    assert!(tree.contains("sheet-close"), "{tree}");
}

#[gpui::test]
fn welcome_is_a_sheet(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe_settled(
        cx,
        &staged,
        "frame/welcome",
        fixture(false, props(), updates("current", "current")),
        None,
    )
    .unwrap();
    assert!(tree.contains("Welcome to Kompitum"), "{tree}");
    assert!(tree.contains("sheet-close"), "{tree}");
}

#[gpui::test]
fn whats_new_is_a_sheet_with_got_it_in_its_footer(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe(
        cx,
        &staged,
        "frame/whatsnew",
        false,
        props(),
        updates("current", "current"),
        None,
    )
    .unwrap();
    assert!(tree.contains("What's new"), "{tree}");
    assert!(tree.contains("sheet-close"), "{tree}");
    assert!(tree.contains("Got it"), "{tree}");
}

#[gpui::test]
fn shortcuts_is_a_sheet(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe(
        cx,
        &staged,
        "mods/help",
        false,
        props(),
        updates("current", "current"),
        Some("this.filter = 'no-such-key';"),
    )
    .unwrap();
    assert!(tree.contains("Keyboard shortcuts"), "{tree}");
    assert!(tree.contains("sheet-close"), "{tree}");
}

#[gpui::test]
fn add_a_machine_is_a_sheet(cx: &mut TestAppContext) {
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
    assert!(tree.contains("Add a machine"), "{tree}");
    assert!(tree.contains("sheet-close"), "{tree}");
}

#[gpui::test]
fn the_mailbox_is_a_sheet(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe_settled(
        cx,
        &staged,
        "frame/mailbox",
        fixture(true, props(), updates("current", "current")),
        None,
    )
    .unwrap();
    assert!(tree.contains("Mailbox"), "{tree}");
    assert!(tree.contains("sheet-close"), "{tree}");
}

#[gpui::test]
fn the_board_is_a_sheet(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe_settled(
        cx,
        &staged,
        "mods/board",
        fixture(true, props(), updates("current", "current")),
        None,
    )
    .unwrap();
    assert!(tree.contains("Board"), "{tree}");
    assert!(tree.contains("sheet-close"), "{tree}");
}

#[gpui::test]
fn the_line_is_a_sheet(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe_settled(
        cx,
        &staged,
        "mods/line",
        fixture(false, props(), updates("current", "current")),
        None,
    )
    .unwrap();
    assert!(tree.contains("Notes"), "{tree}");
    assert!(tree.contains("sheet-close"), "{tree}");
}

/// No sheet or popover says the word "Esc": the app's key closes them, their
/// headers say what they are, and nothing prints a key hint.
#[test]
fn no_sheet_or_popover_prints_esc() {
    let overlays = [
        "frame/settings/main.js",
        "frame/welcome/main.js",
        "frame/whatsnew/main.js",
        "frame/mailbox/main.js",
        "frame/palette/main.js",
        "mods/help/main.js",
        "mods/machine-add/main.js",
        "mods/board/main.js",
        "mods/line/main.js",
        "mods/bell/main.js",
        "mods/limits/main.js",
        "mods/menu/main.js",
        "mods/new-menu/main.js",
        "mods/picker/main.js",
        "mods/prompt/main.js",
        "mods/confirm/main.js",
    ];
    for (path, bytes) in DEFAULT_MODS.iter().chain(FRAME_MODS) {
        let path = *path;
        if !overlays.contains(&path) {
            continue;
        }
        let text = std::str::from_utf8(bytes).unwrap();
        for line in text.lines() {
            let code = line.split("//").next().unwrap_or("");
            let words: Vec<&str> = code
                .split(|c: char| !c.is_alphanumeric() && c != '_')
                .filter(|word| !word.is_empty())
                .collect();
            assert!(
                !words.contains(&"Esc"),
                "the word Esc is printed by {path}: {line}"
            );
        }
    }
}
