use super::*;

fn texts(blocks: &[MenuBlock]) -> Vec<&str> {
    blocks.iter().map(|block| block.text.as_str()).collect()
}

#[test]
fn the_trust_prompt_reads_as_two_rows_with_the_safety_paragraph() {
    let menu = menu_from_screen(include_str!("fixtures/trust.txt")).expect("trust prompt");
    assert_eq!(menu.title, "Accessing workspace:");
    let labels: Vec<&str> = menu.rows.iter().map(|row| row.label.as_str()).collect();
    assert_eq!(labels, ["No, exit", "Yes, I trust this folder"]);
    assert_eq!(menu.pointer, 0);
    let actions: Vec<(&str, &str)> = menu.actions.iter().map(|a| (a.key.as_str(), a.label.as_str())).collect();
    assert_eq!(actions, [("enter", "Confirm"), ("esc", "Cancel")]);
    assert!(
        texts(&menu.body).iter().any(|text| text.contains(r"C:\Users\owner\Projects\demo")),
        "the folder path is in the body: {:?}",
        menu.body
    );
    assert!(
        texts(&menu.body).iter().any(|text| text.starts_with("Quick safety check:")),
        "the safety paragraph is in the body: {:?}",
        menu.body
    );
}

#[test]
fn the_model_menu_reads_its_rows_details_and_note() {
    let menu = menu_from_screen(include_str!("fixtures/model.txt")).expect("model menu");
    assert_eq!(menu.title, "Select model");
    assert_eq!(menu.rows.len(), 6);
    assert_eq!(menu.rows.last().map(|row| row.label.as_str()), Some("More models"));
    assert_eq!(menu.pointer, 4);
    let haiku = &menu.rows[4];
    assert_eq!(haiku.label, "Haiku 5.5");
    assert!(haiku.current);
    assert_eq!(haiku.detail, "Fastest for quick answers");
    let default = &menu.rows[0];
    assert_eq!(default.label, "Default (recommended)");
    assert_eq!(default.detail, "Opus 5.5 · Best for everyday, complex tasks");
    let actions: Vec<(&str, &str)> = menu.actions.iter().map(|a| (a.key.as_str(), a.label.as_str())).collect();
    assert_eq!(
        actions,
        [
            ("enter", "Set as default"),
            ("s", "Use this session only"),
            ("esc", "Cancel")
        ]
    );
    assert!(
        texts(&menu.notes).iter().any(|text| text.contains("◐ Medium effort (default)")),
        "a note reads the effort: {:?}",
        menu.notes
    );
}

#[test]
fn the_theme_menu_reads_eight_rows_and_keeps_its_code_preview_mono() {
    let menu = menu_from_screen(include_str!("fixtures/theme.txt")).expect("theme menu");
    assert_eq!(menu.title, "Theme");
    assert_eq!(menu.rows.len(), 8);
    assert_eq!(menu.pointer, 1);
    assert_eq!(menu.rows[1].label, "Dark mode");
    assert!(menu.rows[1].current);
    assert!(!menu.rows[0].current);
    assert!(
        menu.notes.iter().any(|block| block.mono && block.text.contains("function greet()")),
        "the code preview is a mono note: {:?}",
        menu.notes
    );
}

#[test]
fn the_permission_dialog_reads_its_mono_body_and_rows() {
    let menu = menu_from_screen(include_str!("fixtures/permission.txt")).expect("permission dialog");
    assert_eq!(menu.title, "Create file");
    assert!(
        menu.body.iter().any(|block| block.mono && block.text == "1 hi"),
        "the diff is a mono block: {:?}",
        menu.body
    );
    assert!(
        texts(&menu.body).iter().any(|text| *text == "Do you want to create menu-check.txt?"),
        "the question paragraph is in the body: {:?}",
        menu.body
    );
    assert_eq!(menu.rows.len(), 3);
    assert_eq!(menu.rows[0].label, "Yes");
    assert_eq!(menu.rows[2].label, "No");
    assert_eq!(menu.pointer, 0);
    let actions: Vec<(&str, &str)> = menu.actions.iter().map(|a| (a.key.as_str(), a.label.as_str())).collect();
    assert_eq!(actions, [("enter", ""), ("esc", "Cancel")]);
}

#[test]
fn the_rewind_menu_finds_the_current_row() {
    let menu = menu_from_screen(include_str!("fixtures/rewind.txt")).expect("rewind menu");
    assert_eq!(menu.rows[menu.pointer].label, "(current)");
}

#[test]
fn the_hooks_menu_reads_view_and_close_actions() {
    let menu = menu_from_screen(include_str!("fixtures/hooks.txt")).expect("hooks menu");
    let actions: Vec<(&str, &str)> = menu.actions.iter().map(|a| (a.key.as_str(), a.label.as_str())).collect();
    assert_eq!(actions, [("enter", "View"), ("esc", "Close")]);
}

#[test]
fn the_press_plan_walks_toward_the_row() {
    assert_eq!(menu_steps(0, 3), ("down", 3));
    assert_eq!(menu_steps(5, 2), ("up", 3));
    assert_eq!(menu_steps(4, 4), ("", 0));
}

#[test]
fn screens_that_are_not_one_menu_are_refused() {
    for (name, screen) in [
        ("memory", include_str!("fixtures/memory.txt")),
        ("mcp", include_str!("fixtures/mcp.txt")),
        ("resume", include_str!("fixtures/resume.txt")),
        ("permissions", include_str!("fixtures/permissions.txt")),
        ("config", include_str!("fixtures/config.txt")),
        ("idle", include_str!("fixtures/idle.txt")),
        ("working", include_str!("fixtures/working.txt")),
    ] {
        assert_eq!(menu_from_screen(screen), None, "{name} is not a menu");
    }
}
