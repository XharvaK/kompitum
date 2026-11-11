//! Render tests for the welcome (the UX audit of 2026-11-11): see the parent module for the helpers.
#![allow(unused_imports)]

use super::*;

/// An installed program with no sign-in check says Installed, and offers no Sign in.
#[gpui::test]
fn welcome_installed_program_without_a_sign_in_check_says_installed(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut agents = welcome_agents();
    agents[0]["installed"] = json!(true);
    let tree = describe_settled(
        cx,
        &staged,
        "frame/welcome",
        welcome_values(json!({"lead": {"program": "opencode"}}), agents),
        Some("this.step = 1; this.signedIn = {};"),
    )
    .unwrap();
    assert!(tree.contains("Installed"), "{tree}");
    assert!(!tree.contains("w-signin-opencode"), "no check, so no action: {tree}");
}

/// Step 1 counts no steps and has no Back: the title already says where the person is.
#[gpui::test]
fn welcome_step_one_has_no_counter_and_no_back(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe_settled(
        cx,
        &staged,
        "frame/welcome",
        welcome_values(json!({"lead": {"program": "opencode"}}), welcome_agents()),
        Some("this.step = 1;"),
    )
    .unwrap();
    assert!(!tree.contains("1 of 3"), "{tree}");
    assert!(!tree.contains("w-back"), "{tree}");
    assert!(!tree.contains("Back"), "{tree}");
}

#[gpui::test]
fn welcome_step_one_title_asks_who_to_talk_to(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let tree = describe_settled(
        cx,
        &staged,
        "frame/welcome",
        welcome_values(json!({"lead": {"program": "opencode"}}), welcome_agents()),
        Some("this.step = 1;"),
    )
    .unwrap();
    assert!(tree.contains("Who do you want to talk to?"), "{tree}");
}

/// With a lead other than Claude Code there are two steps, and the second one's button is Start.
#[gpui::test]
fn welcome_with_another_lead_the_second_step_ends_in_start(cx: &mut TestAppContext) {
    cx.update(gpui_shell::init);
    let staged = Staged::new();
    let mut agents = welcome_agents();
    agents[0]["installed"] = json!(true);
    let tree = describe_settled(
        cx,
        &staged,
        "frame/welcome",
        welcome_values(json!({"lead": {"program": "opencode"}}), agents),
        Some("this.step = 2;"),
    )
    .unwrap();
    assert!(tree.contains("w-start"), "{tree}");
    assert!(!tree.contains("w-next"), "the last step has no Continue: {tree}");
}
